//! 面板的一个分类。

use super::PanelGroup;
use crate::candidate::Candidate;

/// 一个分类：所在分组、名字与条目。条目是现成的候选（种类是 emoji / 颜文字 / 符号，`reading` 是中文名字，可能没有），
/// 壳拿去排格子、上屏走 [`crate::Engine::commit`]。
#[derive(Debug, Clone)]
pub struct PanelSection {
    pub group: PanelGroup,

    pub name: String,

    pub items: Vec<Candidate>,
}
