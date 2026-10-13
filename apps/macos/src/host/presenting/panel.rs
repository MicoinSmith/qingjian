//! 符号面板的呈现：当前分类的条目排成格子，顶上两行标签，信息行放高亮条目的名字。

use qingjian_core::{CandidateKind, PanelSection};

use crate::candidates::{Frame, PanelTabs, Row};
use crate::host::Host;

/// 一行几格按分类里最宽的条目定：符号与 emoji 一格一字，十格一行；颜文字宽，少放几格（面板总宽不变，格子拉宽）。
const NARROW_COLUMNS: usize = 10;
const MEDIUM_COLUMNS: usize = 6;
const WIDE_COLUMNS: usize = 4;

/// 估宽时一条最多算几个字宽：再宽的在格子里截断，信息行里看全文。
const MAX_ITEM_EMS: f32 = 8.0;

impl Host {
    /// 面板刚打开或换了分类：格子换成当前分类的条目，高亮回到第一格。
    pub fn load_panel(&mut self) {
        let Some(section) = self.engine.symbol_panel().map(|p| p.section().clone()) else {
            return;
        };
        let ems = item_ems(&section);
        let columns = if ems <= 2.2 {
            NARROW_COLUMNS
        } else if ems <= 5.0 {
            MEDIUM_COLUMNS
        } else {
            WIDE_COLUMNS
        };
        self.session.open_panel(section.items, columns);
    }

    /// 面板这一帧：视口里的几行格子与标签。格子不标序号，选择靠方向键加空格 / 回车。
    pub(super) fn render_panel(&mut self) {
        let Some(panel) = self.engine.symbol_panel() else {
            return;
        };
        let groups = panel.groups();
        let tabs = PanelTabs {
            groups: groups.iter().map(|g| g.label().to_owned()).collect(),
            group: groups.iter().position(|g| *g == panel.group()).unwrap_or(0),
            sections: panel
                .group_sections()
                .0
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            section: panel.group_sections().1,
        };
        let Some((cells, columns)) = self.session.grid_cells() else {
            return;
        };
        let top = self.session.grid.map_or(0, |grid| grid.top());
        let rows = cells
            .iter()
            .map(|cell| match cell.candidate() {
                Some(candidate) => {
                    let mut row = Row::from_candidate(0, candidate);
                    row.index = String::new();
                    row
                }
                None => Row {
                    index: String::new(),
                    text: String::new(),
                    annotation: Vec::new(),
                    cloud: false,
                },
            })
            .collect();
        let size = self.session.layout.page_size();
        let pages = self.session.pages();
        let frame = Frame {
            preedit: None,
            rows,
            highlighted: self.session.highlighted.saturating_sub(top * size),
            columns,
            column_ems: Vec::new(),
            footer: (pages > 1).then(|| format!("{}/{pages}", self.session.page + 1)),
            sentence: None,
            status: None,
            mode: self.input_mode(),
            panel: Some(tabs),
        };
        self.window.show(frame, self.anchor);
    }
}

/// 分类里最宽的一条按几个字宽估：emoji 一个字宽，ASCII 半个，组合附加符号不占宽。
fn item_ems(section: &PanelSection) -> f32 {
    section
        .items
        .iter()
        .map(|item| {
            if item.kind == CandidateKind::Emoji {
                return 1.0;
            }
            item.text
                .chars()
                .map(|c| match c {
                    '\u{0300}'..='\u{036F}' | '\u{FE00}'..='\u{FE0F}' => 0.0,
                    c if c.is_ascii() => 0.6,
                    _ => 1.0,
                })
                .sum::<f32>()
        })
        .fold(1.0_f32, f32::max)
        .min(MAX_ITEM_EMS)
}
