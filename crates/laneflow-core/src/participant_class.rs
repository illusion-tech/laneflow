//! ParticipantClass 参与者分类 taxonomy normalization。

use indexmap::IndexMap;

use crate::{
    error::CoreError, handle::ParticipantClassHandle, id::validate_external_id,
    junction::validate_capacity,
};

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
/// normalization 编译 per-class 继承深度与子树区间（Euler tour 半开区间
/// `[enter, exit)`：层级为无环单继承森林，`descendant ∈ subtree(ancestor)`
/// 当且仅当 `enter[ancestor] ≤ enter[descendant] < exit[ancestor]`），层级匹配
/// 在绑定期为 O(1) 区间包含查询，不进入字符串比较。存储 O(classes)——此前的
/// descendants bitset 是 O(classes²/64)，不可信输入下 10 万个 class 就要
/// 分配 ~1.25 GB 并付出平方级初始化时间（容量校验只约束 class 数量本身）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticipantClassRegistry {
    classes: Vec<ResolvedParticipantClass>,
    handles: IndexMap<String, ParticipantClassHandle>,
    /// 每 class 的 Euler tour 半开子树区间 `(enter, exit)`（exit = enter + 子树大小）。
    subtree_intervals: Vec<(u32, u32)>,
}

impl ParticipantClassRegistry {
    /// 创建空 ParticipantClass registry。
    pub fn empty() -> Self {
        Self {
            classes: Vec::new(),
            handles: IndexMap::new(),
            subtree_intervals: Vec::new(),
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
        let subtree_intervals = compile_subtree_intervals(&parents);

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
            subtree_intervals,
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

    /// 判断 `descendant` 是否等于或为 `ancestor` 的传递后代（O(1) 区间包含查询）。
    /// 任一 handle 不属于本 registry 时返回 `false`。
    pub fn is_descendant_or_self(
        &self,
        descendant: ParticipantClassHandle,
        ancestor: ParticipantClassHandle,
    ) -> bool {
        let Some(&(descendant_enter, _)) = self.subtree_intervals.get(descendant.index()) else {
            return false;
        };
        let Some(&(ancestor_enter, ancestor_exit)) = self.subtree_intervals.get(ancestor.index())
        else {
            return false;
        };
        ancestor_enter <= descendant_enter && descendant_enter < ancestor_exit
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        let Self {
            classes,
            handles,
            subtree_intervals,
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
        let interval_bytes = subtree_intervals.capacity() * std::mem::size_of::<(u32, u32)>();

        class_bytes + resolver_bytes + interval_bytes
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

/// 为无环单继承森林编译 Euler tour 半开子树区间 `(enter, exit)`：
/// `exit = enter + 子树大小`，`d ∈ subtree(a)` ⟺ `enter[a] ≤ enter[d] < exit[a]`。
/// 存储与初始化都是 O(classes)（迭代 DFS，深继承链不递归），替代平方级
/// descendants bitset。
fn compile_subtree_intervals(parents: &[Option<ParticipantClassHandle>]) -> Vec<(u32, u32)> {
    let mut children: Vec<Vec<u32>> = vec![Vec::new(); parents.len()];
    for (child, parent) in parents.iter().enumerate() {
        if let Some(parent) = parent {
            children[parent.index()].push(
                u32::try_from(child).expect("class count 已 validate_capacity 约束在 u32 范围"),
            );
        }
    }
    let mut intervals = vec![(0_u32, 0_u32); parents.len()];
    let mut next_enter: u32 = 0;
    // 栈元素为 (节点, 下一个待访问子节点下标)；enter 在首次访问赋值，
    // exit 在子树遍历完赋值为当前 next_enter（= enter + 子树大小）。
    for root in 0..parents.len() {
        if parents[root].is_some() {
            continue;
        }
        let mut stack = vec![(
            u32::try_from(root).expect("class count 已 validate_capacity 约束在 u32 范围"),
            0_usize,
        )];
        while let Some(&(node, cursor)) = stack.last() {
            if cursor == 0 {
                intervals[node as usize].0 = next_enter;
                next_enter += 1;
            }
            if cursor < children[node as usize].len() {
                let child = children[node as usize][cursor];
                stack.last_mut().expect("stack 非空").1 += 1;
                stack.push((child, 0));
            } else {
                intervals[node as usize].1 = next_enter;
                stack.pop();
            }
        }
    }
    intervals
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

    #[test]
    fn hierarchy_matching_uses_subtree_intervals() {
        let registry = ParticipantClassRegistry::try_new(vec![
            ParticipantClass::new("motorVehicle", None),
            ParticipantClass::new("car", Some("motorVehicle")),
            ParticipantClass::new("taxi", Some("car")),
            ParticipantClass::new("bicycle", None),
        ])
        .expect("valid class registry");
        let handle = |id: &str| registry.class_handle(id).expect("declared class");

        // 自身与传递后代为 true；祖先、兄弟、无关节点为 false。
        assert!(registry.is_descendant_or_self(handle("taxi"), handle("taxi")));
        assert!(registry.is_descendant_or_self(handle("taxi"), handle("car")));
        assert!(registry.is_descendant_or_self(handle("taxi"), handle("motorVehicle")));
        assert!(!registry.is_descendant_or_self(handle("motorVehicle"), handle("taxi")));
        assert!(!registry.is_descendant_or_self(handle("taxi"), handle("bicycle")));
        assert!(!registry.is_descendant_or_self(handle("bicycle"), handle("motorVehicle")));
        // 越界 handle 按 false 处理。
        assert!(!registry.is_descendant_or_self(
            ParticipantClassHandle::new(u32::MAX as usize),
            handle("motorVehicle")
        ));
    }
}
