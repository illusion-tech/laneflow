//! 注册时建立的紧凑路线表：观测热路径按 harness 自有序号取下标，不走 SipHash。

use laneflow_runtime::RouteHandle;
use laneflow_static_contract::LaneEdgeOrdinal;

use crate::fastmap::FastMap;

/// 注册顺序即目录 key 升序，因此自有序号排序与 key 排序等价（claims 依赖这一点）。
#[derive(Default)]
pub(crate) struct RouteTable {
    handles: Vec<RouteHandle>,
    keys: Vec<String>,
    edge_keys: Vec<Vec<String>>,
    edge_raws: Vec<Vec<u32>>,
    edge_len_mm: Vec<Vec<u32>>,
    /// 每路线 `edge_count - 1` 项：hop 从边 hop 进入边 hop+1 是否跨 tile。
    cross_tile: Vec<Vec<bool>>,
    max_edge_raw: u32,
    index_of: FastMap<RouteHandle, u32>,
}

impl RouteTable {
    pub(crate) fn register(
        &mut self,
        handle: RouteHandle,
        key: String,
        edge_keys: Vec<String>,
        edges: &[LaneEdgeOrdinal],
        lengths: &[u32],
    ) {
        let index = self.handles.len() as u32;
        self.handles.push(handle);
        self.keys.push(key);
        self.edge_raws.push(edges.iter().map(|e| e.raw()).collect());
        self.max_edge_raw = edges
            .iter()
            .fold(self.max_edge_raw, |max, e| max.max(e.raw()));
        self.edge_len_mm
            .push(edges.iter().map(|e| lengths[e.index()]).collect());
        self.cross_tile.push(
            edge_keys
                .windows(2)
                .map(|pair| pair[0].split('.').next() != pair[1].split('.').next())
                .collect(),
        );
        self.edge_keys.push(edge_keys);
        self.index_of.insert(handle, index);
    }

    pub(crate) fn index(&self, handle: RouteHandle) -> u32 {
        self.index_of[&handle]
    }

    pub(crate) fn handle(&self, index: u32) -> RouteHandle {
        self.handles[index as usize]
    }

    pub(crate) fn key(&self, index: u32) -> &str {
        &self.keys[index as usize]
    }

    pub(crate) fn key_of(&self, handle: RouteHandle) -> &str {
        self.key(self.index(handle))
    }

    pub(crate) fn edge_count(&self, index: u32) -> usize {
        self.edge_raws[index as usize].len()
    }

    pub(crate) fn edge_raw(&self, index: u32, hop: usize) -> u32 {
        self.edge_raws[index as usize][hop]
    }

    pub(crate) fn edge_len_mm(&self, index: u32, hop: usize) -> u32 {
        self.edge_len_mm[index as usize][hop]
    }

    pub(crate) fn edge_key(&self, index: u32, hop: usize) -> &str {
        &self.edge_keys[index as usize][hop]
    }

    #[cfg(test)]
    pub(crate) fn edge_keys_of(&self, handle: RouteHandle) -> &[String] {
        &self.edge_keys[self.index(handle) as usize]
    }

    pub(crate) fn crosses_tile(&self, index: u32, hop: usize) -> bool {
        self.cross_tile[index as usize][hop]
    }

    /// `edge_raw` 取值上界（最大 raw + 1），供按边直接索引的桶结构定界。
    pub(crate) fn edge_index_capacity(&self) -> usize {
        self.max_edge_raw as usize + 1
    }

    /// 快照恢复后按旧句柄→新句柄重绑定；路线内容与序号不变。
    #[cfg(feature = "adapter")]
    pub(crate) fn remap_handles(
        &mut self,
        map: &std::collections::HashMap<RouteHandle, RouteHandle>,
    ) {
        for handle in &mut self.handles {
            *handle = map[handle];
        }
        self.index_of = (0..self.handles.len() as u32)
            .map(|index| (self.handles[index as usize], index))
            .collect();
    }
}
