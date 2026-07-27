//! ParticipantClass 参与者分类 taxonomy normalization。

use indexmap::IndexMap;

use crate::{
    error::CoreError, handle::ParticipantClassHandle, id::validate_external_id,
    junction::validate_capacity,
};

const BITS_PER_BLOCK: usize = u64::BITS as usize;

/// ParticipantClass 输入定义（单继承）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticipantClass {
    id: String,
    extends_id: Option<String>,
}

impl ParticipantClass {
    /// 创建 ParticipantClass。ID 语法、唯一性、extends 引用与继承环由
    /// `ParticipantClassRegistry::try_new` 校验。
    pub fn new(id: impl Into<String>, extends_id: Option<&str>) -> Self {
        Self {
            id: id.into(),
            extends_id: extends_id.map(str::to_owned),
        }
    }

    /// 返回 ParticipantClass external ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回单继承父类 external ID。
    pub fn extends_id(&self) -> Option<&str> {
        self.extends_id.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedParticipantClass {
    definition: ParticipantClass,
    parent: Option<ParticipantClassHandle>,
    depth: u32,
}

/// ParticipantClass immutable normalized registry。
///
/// normalization 编译 per-class 继承深度与 descendants bitset（标记自身与全部传递
/// 后代），层级匹配在绑定期为 O(1) bitset 查询，不进入字符串比较。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticipantClassRegistry {
    classes: Vec<ResolvedParticipantClass>,
    handles: IndexMap<String, ParticipantClassHandle>,
    descendant_blocks: Vec<u64>,
    blocks_per_class: usize,
}

impl ParticipantClassRegistry {
    /// 创建空 ParticipantClass registry。
    pub fn empty() -> Self {
        Self {
            classes: Vec::new(),
            handles: IndexMap::new(),
            descendant_blocks: Vec::new(),
            blocks_per_class: 0,
        }
    }

    /// 创建并校验 ParticipantClass taxonomy。
    ///
    /// 校验按 phase 顺序进行，同 phase 内按 input order 返回首错：
    /// 1. ID syntax/duplicate；
    /// 2. unknown `extendsId`（引用必须解析到已声明 class）；
    /// 3. 继承环检测（无环单继承）。
    pub fn try_new(classes: Vec<ParticipantClass>) -> Result<Self, CoreError> {
        validate_capacity("participantClasses", classes.len())?;

        let mut handles = IndexMap::new();
        for (index, class) in classes.iter().enumerate() {
            validate_external_id("participantClasses[].id", class.id())?;
            if handles.contains_key(class.id()) {
                return Err(CoreError::DuplicateParticipantClassId {
                    class_id: class.id().to_owned(),
                });
            }
            handles.insert(class.id().to_owned(), ParticipantClassHandle::new(index));
        }

        let mut parents = Vec::with_capacity(classes.len());
        for class in &classes {
            let parent = match class.extends_id() {
                Some(extends_id) => {
                    validate_external_id("participantClasses[].extendsId", extends_id)?;
                    Some(handles.get(extends_id).copied().ok_or_else(|| {
                        CoreError::UnknownParticipantClassExtends {
                            class_id: class.id().to_owned(),
                            extends_id: extends_id.to_owned(),
                        }
                    })?)
                }
                None => None,
            };
            parents.push(parent);
        }

        detect_inheritance_cycle(&classes, &parents)?;

        let depths = compile_depths(&parents);
        let blocks_per_class = classes.len().div_ceil(BITS_PER_BLOCK);
        let descendant_blocks =
            compile_descendant_blocks(&parents, classes.len(), blocks_per_class);

        let classes = classes
            .into_iter()
            .zip(parents)
            .zip(depths)
            .map(|((definition, parent), depth)| ResolvedParticipantClass {
                definition,
                parent,
                depth,
            })
            .collect();

        Ok(Self {
            classes,
            handles,
            descendant_blocks,
            blocks_per_class,
        })
    }

    /// 返回 registry 是否为空。
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }

    /// 返回 class 数量。
    pub fn class_count(&self) -> usize {
        self.classes.len()
    }

    /// 返回 ParticipantClass external ID 对应的 handle。
    pub fn class_handle(&self, external_id: &str) -> Option<ParticipantClassHandle> {
        self.handles.get(external_id).copied()
    }

    /// 返回 ParticipantClass handle 对应的 external ID。
    pub fn class_external_id(&self, handle: ParticipantClassHandle) -> Option<&str> {
        self.class(handle).map(ParticipantClass::id)
    }

    /// 返回指定 ParticipantClass definition。
    pub fn class(&self, handle: ParticipantClassHandle) -> Option<&ParticipantClass> {
        self.classes
            .get(handle.index())
            .map(|resolved| &resolved.definition)
    }

    /// 按 normalization order 遍历 ParticipantClass handles。
    pub fn classes(&self) -> impl ExactSizeIterator<Item = ParticipantClassHandle> + '_ {
        (0..self.classes.len()).map(ParticipantClassHandle::new)
    }

    /// 返回 class 的继承深度（root 为 0，用于参与者 specificity 比较）。
    pub fn depth(&self, handle: ParticipantClassHandle) -> Option<u32> {
        self.classes.get(handle.index()).map(|class| class.depth)
    }

    /// 判断 `descendant` 是否等于或为 `ancestor` 的传递后代（O(1) bitset 查询）。
    /// 任一 handle 不属于本 registry 时返回 `false`。
    pub fn is_descendant_or_self(
        &self,
        descendant: ParticipantClassHandle,
        ancestor: ParticipantClassHandle,
    ) -> bool {
        if descendant.index() >= self.classes.len() || ancestor.index() >= self.classes.len() {
            return false;
        }
        let block = self.descendant_blocks
            [ancestor.index() * self.blocks_per_class + descendant.index() / BITS_PER_BLOCK];
        block & (1_u64 << (descendant.index() % BITS_PER_BLOCK)) != 0
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        let Self {
            classes,
            handles,
            descendant_blocks,
            blocks_per_class: _,
        } = self;

        let class_bytes = classes.capacity() * std::mem::size_of::<ResolvedParticipantClass>()
            + classes
                .iter()
                .map(|class| {
                    class.definition.id.capacity()
                        + class
                            .definition
                            .extends_id
                            .as_ref()
                            .map_or(0, String::capacity)
                })
                .sum::<usize>();
        let resolver_bytes = handles.capacity()
            * std::mem::size_of::<(String, ParticipantClassHandle)>()
            + handles.keys().map(String::capacity).sum::<usize>();
        let bitset_bytes = descendant_blocks.capacity() * std::mem::size_of::<u64>();

        class_bytes + resolver_bytes + bitset_bytes
    }
}

fn detect_inheritance_cycle(
    classes: &[ParticipantClass],
    parents: &[Option<ParticipantClassHandle>],
) -> Result<(), CoreError> {
    // 三色标记：0 = 未访问，1 = 当前链上，2 = 已确认无环。
    let mut states = vec![0_u8; classes.len()];
    for start in 0..classes.len() {
        if states[start] != 0 {
            continue;
        }
        let mut chain = Vec::new();
        let mut current = start;
        loop {
            match states[current] {
                2 => break,
                1 => {
                    return Err(CoreError::ParticipantClassInheritanceCycle {
                        class_id: classes[current].id().to_owned(),
                    });
                }
                _ => {
                    states[current] = 1;
                    chain.push(current);
                    match parents[current] {
                        Some(parent) => current = parent.index(),
                        None => break,
                    }
                }
            }
        }
        for node in chain {
            states[node] = 2;
        }
    }
    Ok(())
}

fn compile_depths(parents: &[Option<ParticipantClassHandle>]) -> Vec<u32> {
    let mut depths = vec![None; parents.len()];
    for start in 0..parents.len() {
        let mut chain = Vec::new();
        let mut current = start;
        let mut next_depth = loop {
            if let Some(depth) = depths[current] {
                // memo 命中：chain 尾节点是已知节点的直接子类。
                break depth + 1;
            }
            chain.push(current);
            match parents[current] {
                Some(parent) => current = parent.index(),
                // chain 尾节点即 root，深度为 0。
                None => break 0,
            }
        };
        for node in chain.into_iter().rev() {
            depths[node] = Some(next_depth);
            next_depth += 1;
        }
    }
    depths
        .into_iter()
        .map(|depth| depth.expect("无环节点必须获得继承深度"))
        .collect()
}

fn compile_descendant_blocks(
    parents: &[Option<ParticipantClassHandle>],
    class_count: usize,
    blocks_per_class: usize,
) -> Vec<u64> {
    let mut blocks = vec![0_u64; class_count * blocks_per_class];
    for descendant in 0..class_count {
        let bit = 1_u64 << (descendant % BITS_PER_BLOCK);
        let block_index = descendant / BITS_PER_BLOCK;
        blocks[descendant * blocks_per_class + block_index] |= bit;
        let mut current = parents[descendant];
        while let Some(parent) = current {
            blocks[parent.index() * blocks_per_class + block_index] |= bit;
            current = parents[parent.index()];
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_bytes_tracks_declared_classes() {
        let empty = ParticipantClassRegistry::empty();
        let registry = ParticipantClassRegistry::try_new(vec![
            ParticipantClass::new("motorVehicle", None),
            ParticipantClass::new("car", Some("motorVehicle")),
        ])
        .expect("valid class registry");

        assert_eq!(empty.retained_bytes(), 0);
        assert!(registry.retained_bytes() > 0);
    }
}
