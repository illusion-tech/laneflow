//! #678：固定 live 序和精确行集合上的有限结构实验，不是 Runtime 候选实现。
use std::hint::black_box;
use std::mem::size_of;
use std::time::Instant;

const EDGES: usize = 64;
const BLOCK: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Row {
    handle: u64,
    edge: u32,
    lo: u32,
    hi: u32,
    order: u32,
}

fn key(row: Row) -> (u32, u32, u32, u64) {
    (row.edge, row.lo, row.order, row.handle)
}

struct Csr {
    rows: Vec<Row>,
    offsets: Vec<usize>,
}

impl Csr {
    fn new(rows: Vec<Row>) -> Self {
        let mut offsets = vec![0; EDGES + 1];
        for row in &rows {
            offsets[row.edge as usize + 1] += 1;
        }
        for i in 1..offsets.len() {
            offsets[i] += offsets[i - 1];
        }
        let mut positions = offsets.clone();
        let mut ordered = vec![
            Row {
                handle: 0,
                edge: 0,
                lo: 0,
                hi: 0,
                order: 0
            };
            rows.len()
        ];
        for row in rows {
            let edge = row.edge as usize;
            ordered[positions[edge]] = row;
            positions[edge] += 1;
        }
        for edge in 0..EDGES {
            ordered[offsets[edge]..offsets[edge + 1]].sort_unstable_by_key(|r| key(*r));
        }
        Self {
            rows: ordered,
            offsets,
        }
    }

    fn first(&self, edge: usize) -> Option<Row> {
        self.rows[self.offsets[edge]..self.offsets[edge + 1]]
            .first()
            .copied()
    }

    fn bytes(&self) -> usize {
        self.rows.capacity() * size_of::<Row>() + self.offsets.capacity() * size_of::<usize>()
    }
}

enum Index {
    Rebuild(Csr),
    Local(Vec<Vec<Row>>),
    Overlay { base: Csr, delta: Vec<Vec<Row>> },
}

fn bucket_bytes(buckets: &Vec<Vec<Row>>) -> usize {
    buckets.capacity() * size_of::<Vec<Row>>()
        + buckets
            .iter()
            .map(|b| b.capacity() * size_of::<Row>())
            .sum::<usize>()
}

impl Index {
    fn new(mode: usize, rows: Vec<Row>) -> Self {
        match mode {
            0 => Self::Rebuild(Csr::new(rows)),
            1 => {
                let mut buckets = vec![Vec::new(); EDGES];
                for row in rows {
                    buckets[row.edge as usize].push(row);
                }
                for bucket in &mut buckets {
                    bucket.sort_unstable_by_key(|r| key(*r));
                }
                Self::Local(buckets)
            }
            2 => Self::Overlay {
                base: Csr::new(rows),
                delta: vec![Vec::new(); EDGES],
            },
            _ => unreachable!(),
        }
    }

    /// 返回显式重写/平移行数；排序器内部搬移不计入这个下界。
    fn insert(&mut self, row: Row) -> usize {
        match self {
            Self::Rebuild(base) => {
                let mut rows = base.rows.clone();
                rows.push(row);
                let writes = rows.len();
                *base = Csr::new(rows);
                writes
            }
            Self::Local(buckets) | Self::Overlay { delta: buckets, .. } => {
                let bucket = &mut buckets[row.edge as usize];
                let index = bucket.partition_point(|r| key(*r) < key(row));
                let writes = bucket.len() - index + 1;
                bucket.insert(index, row);
                writes
            }
        }
    }

    fn first(&self, edge: usize) -> Option<Row> {
        match self {
            Self::Rebuild(base) => base.first(edge),
            Self::Local(buckets) => buckets[edge].first().copied(),
            Self::Overlay { base, delta } => base
                .first(edge)
                .into_iter()
                .chain(delta[edge].first().copied())
                .min_by_key(|r| key(*r)),
        }
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = match self {
            Self::Rebuild(base) => base.rows.clone(),
            Self::Local(buckets) => buckets.iter().flatten().copied().collect(),
            Self::Overlay { base, delta } => base
                .rows
                .iter()
                .chain(delta.iter().flatten())
                .copied()
                .collect(),
        };
        rows.sort_unstable_by_key(|r| key(*r));
        rows
    }

    fn bytes(&self) -> usize {
        match self {
            Self::Rebuild(base) => base.bytes(),
            Self::Local(buckets) => bucket_bytes(buckets),
            Self::Overlay { base, delta } => base.bytes() + bucket_bytes(delta),
        }
    }
}

struct Active {
    mode: usize,
    flags: Vec<bool>,
    output: Vec<usize>,
    bits: Vec<u64>,
    blocks: Vec<Vec<usize>>,
}

impl Active {
    fn new(mode: usize, active: usize, parked: usize) -> Self {
        let live = active + parked;
        let mut flags = vec![false; live];
        flags[parked..].fill(true);
        let mut output = Vec::with_capacity(live);
        output.extend(parked..live);
        let bits = if mode == 2 {
            let mut bits = vec![0; live.div_ceil(64)];
            for rank in parked..live {
                bits[rank / 64] |= 1 << (rank % 64);
            }
            bits
        } else {
            Vec::new()
        };
        let blocks = if mode == 3 {
            let mut blocks = vec![Vec::new(); live.div_ceil(BLOCK)];
            for rank in parked..live {
                blocks[rank / BLOCK].push(rank);
            }
            blocks
        } else {
            Vec::new()
        };
        Self {
            mode,
            flags,
            output,
            bits,
            blocks,
        }
    }

    /// 各次提交后都物化与现行 Active Vec 等价的顺序；不隐藏查询端成本。
    fn activate(&mut self, rank: usize) -> usize {
        self.flags[rank] = true;
        if self.mode == 1 {
            let index = self.output.partition_point(|v| *v < rank);
            let writes = self.output.len() - index + 1;
            self.output.insert(index, rank);
            return writes;
        }
        self.output.clear();
        match self.mode {
            0 => self.output.extend(
                self.flags
                    .iter()
                    .enumerate()
                    .filter_map(|(i, active)| active.then_some(i)),
            ),
            2 => {
                self.bits[rank / 64] |= 1 << (rank % 64);
                for (i, &word) in self.bits.iter().enumerate() {
                    let mut word = word;
                    while word != 0 {
                        self.output.push(i * 64 + word.trailing_zeros() as usize);
                        word &= word - 1;
                    }
                }
            }
            3 => {
                let block = &mut self.blocks[rank / BLOCK];
                let index = block.partition_point(|v| *v < rank);
                block.insert(index, rank);
                for block in &self.blocks {
                    self.output.extend_from_slice(block);
                }
            }
            _ => unreachable!(),
        }
        self.output.len()
    }

    /// 公共已提交 flags 作为输入，不计入候选自有缓冲；未建身份/rank 映射。
    fn bytes(&self) -> usize {
        self.output.capacity() * size_of::<usize>()
            + self.bits.capacity() * size_of::<u64>()
            + self.blocks.capacity() * size_of::<Vec<usize>>()
            + self
                .blocks
                .iter()
                .map(|b| b.capacity() * size_of::<usize>())
                .sum::<usize>()
    }
}

fn initial_rows(count: usize, hot: bool) -> Vec<Row> {
    (0..count)
        .map(|i| Row {
            handle: i as u64,
            edge: if hot { 0 } else { (i % EDGES) as u32 },
            lo: 10_000 + i as u32 * 10,
            hi: 10_005 + i as u32 * 10,
            order: i as u32,
        })
        .collect()
}

fn next_row(initial: usize, index: usize, hot: bool) -> Row {
    Row {
        handle: (initial + index) as u64,
        edge: if hot { 0 } else { (index % EDGES) as u32 },
        lo: 5_000 - index as u32 * 10,
        hi: 5_005 - index as u32 * 10,
        order: (initial + index) as u32,
    }
}

#[test]
fn parking_structure_models_preserve_every_prefix() {
    for hot in [false, true] {
        let mut indexes: Vec<_> = (0..3)
            .map(|mode| Index::new(mode, initial_rows(128, hot)))
            .collect();
        for i in 0..64 {
            for index in &mut indexes {
                index.insert(next_row(128, i, hot));
            }
            let expected = indexes[0].rows();
            for index in &indexes {
                assert_eq!(index.rows(), expected);
                for edge in 0..EDGES {
                    assert_eq!(index.first(edge), indexes[0].first(edge));
                }
            }
        }
    }
    for parked in [64, 4_096] {
        let mut active: Vec<_> = (0..4).map(|mode| Active::new(mode, 128, parked)).collect();
        for rank in (0..64).rev() {
            for a in &mut active {
                a.activate(rank);
            }
            for a in &active {
                assert_eq!(a.output, active[0].output);
            }
        }
    }
}

#[test]
#[ignore = "manual storage models; excludes Runtime geometry, failure semantics and identity maintenance"]
fn parking_structure_release_matrix() {
    for initial in [128, 1_024] {
        for hot in [false, true] {
            for queries in [1, 64] {
                for mode in 0..3 {
                    let mut times = Vec::new();
                    let mut retained = 0;
                    let mut writes = 0;
                    for round in 0..36 {
                        let mut index = Index::new(mode, initial_rows(initial, hot));
                        let start = Instant::now();
                        let mut moved = 0;
                        for i in 0..64 {
                            moved += index.insert(next_row(initial, i, hot));
                            for edge in 0..queries {
                                black_box(index.first(edge));
                            }
                        }
                        let ns = start.elapsed().as_nanos();
                        if round >= 4 {
                            times.push(ns);
                        }
                        retained = index.bytes();
                        writes = moved;
                        assert_eq!(index.rows().len(), initial + 64);
                    }
                    println!(
                        "parking-index-model initial={initial} hot={hot} queries={queries} mode={mode} retained={retained} writes={writes} ns={times:?}"
                    );
                }
            }
        }
        for parked in [64, 4_096] {
            for mode in 0..4 {
                let mut times = Vec::new();
                let mut retained = 0;
                let mut writes = 0;
                for round in 0..36 {
                    let mut active = Active::new(mode, initial, parked);
                    let start = Instant::now();
                    let mut moved = 0;
                    for rank in (0..64).rev() {
                        moved += active.activate(rank);
                    }
                    black_box(&active.output);
                    let ns = start.elapsed().as_nanos();
                    if round >= 4 {
                        times.push(ns);
                    }
                    retained = active.bytes();
                    writes = moved;
                    assert_eq!(active.output.len(), initial + 64);
                }
                println!(
                    "parking-active-model initial={initial} parked={parked} mode={mode} retained={retained} writes={writes} ns={times:?}"
                );
            }
        }
    }
}
