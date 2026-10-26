//! 鼠标点候选窗口：点候选与按它的数字键一样上屏，点译词与按译词键一样上屏它的那条译词；右键候选弹菜单。
//!
//! 候选窗口不知道是哪个会话，所以在 `activateServer:` 时记下当前控制器，点击时用它的客户端上屏。

use std::cell::RefCell;

use objc2::Message;
use qingjian_platform::candidate_menu::CandidateAction;
use qingjian_render::HitTarget;

use crate::candidates::PanelMouse;

use super::*;

thread_local! {
    /// 最近激活的会话；`deactivateServer:` 时清掉。
    static ACTIVE: RefCell<Option<Retained<QingjianInputController>>> = const { RefCell::new(None) };
}

/// 候选窗口的点击回调（host 建窗口时注册）。
pub fn click(target: HitTarget) {
    let Some(controller) = ACTIVE.with(|active| active.borrow().clone()) else {
        tracing::debug!("没有激活的会话，点击不处理");
        return;
    };
    if catch_panic("click", || controller.click(target)).is_none() {
        recover_from_panic(None);
    }
}

/// 单击 Shift 切到英文之后：组句中的拼音原样当英文上屏（像搜狗），候选窗口一并收掉。
///
/// 修饰键事件回调里调（`imk/controller/mod.rs` 的 `handle_shift_event`），要借文本客户端，所以放在 Host 借用之外；
/// 没在组句、或不在英文模式时什么都不做。
pub fn commit_raw_after_switch() {
    let commit = crate::host::with(|h| h.english_mode() && !h.engine.composition().is_empty())
        .unwrap_or(false);
    if !commit {
        return;
    }
    tracing::debug!("切到英文：拼音原样上屏");
    with_active(|controller, client| {
        controller.commit_raw(client);
    });
}

/// 符号面板的点击回调（host 建窗口时注册）。
pub fn panel_mouse(hit: PanelMouse) {
    let Some(controller) = ACTIVE.with(|active| active.borrow().clone()) else {
        return;
    };
    // SAFETY: IMKInputController 的 `client` 返回当前会话的文本客户端（实现 IMKTextInput 的代理），可能为 nil
    let client: Option<Retained<AnyObject>> = unsafe { msg_send![&*controller, client] };
    let Some(client) = client else {
        return;
    };
    if catch_panic("panel_click", || {
        controller.panel_mouse_event(hit, TextClient::new(&client))
    })
    .is_none()
    {
        recover_from_panic(None);
    }
}

/// 在当前激活的会话上执行一次「用客户端做事」的操作。
/// 没有激活的会话、或拿不到文本客户端时直接跳过。
fn with_active(f: impl FnOnce(&QingjianInputController, TextClient<'_>)) {
    let Some(controller) = ACTIVE.with(|active| active.borrow().clone()) else {
        tracing::debug!("没有激活的会话，跳过");
        return;
    };
    // SAFETY: 同 `click`：IMKInputController 的 `client` 返回当前会话的文本客户端，可能为 nil
    let client: Option<Retained<AnyObject>> = unsafe { msg_send![&*controller, client] };
    let Some(client) = client else {
        return;
    };
    f(&controller, TextClient::new(&client));
}

/// 候选右键菜单要的项：组句中才给（编号, 标题）。
pub fn menu_items(offset: usize) -> Vec<(u32, &'static str)> {
    let composing = host::with(|h| !h.engine.composition().is_empty()).unwrap_or(false);
    if !composing {
        return Vec::new();
    }
    host::with(|h| h.candidate_actions(offset))
        .unwrap_or_default()
        .into_iter()
        .map(|action| (action.id(), action.label()))
        .collect()
}

/// 候选右键菜单点中了一项。
pub fn menu_pick(offset: usize, id: u32) {
    let Some(action) = CandidateAction::from_id(id) else {
        return;
    };
    let Some(controller) = ACTIVE.with(|active| active.borrow().clone()) else {
        return;
    };
    if catch_panic("candidate_menu", || {
        controller.candidate_menu_pick(offset, action)
    })
    .is_none()
    {
        recover_from_panic(None);
    }
}

impl QingjianInputController {
    pub(super) fn set_active(&self) {
        ACTIVE.with(|active| *active.borrow_mut() = Some(self.retain()));
    }

    pub(super) fn clear_active(&self) {
        ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            if active
                .as_deref()
                .is_some_and(|current| std::ptr::eq(current, self))
            {
                *active = None;
            }
        });
    }

    /// 做右键菜单里的一项：重新查一遍（排序会变），结果那句话显示在拼音行右侧，敲下一键就没了。
    fn candidate_menu_pick(&self, offset: usize, action: CandidateAction) {
        // SAFETY: IMKInputController 的 `client` 返回当前会话的文本客户端（实现 IMKTextInput 的代理），可能为 nil
        let client: Option<Retained<AnyObject>> = unsafe { msg_send![self, client] };
        let Some(client) = client else {
            return;
        };
        let client = TextClient::new(&client);
        let Some(message) = host::with(|h| h.perform_candidate_action(offset, action)).flatten()
        else {
            return;
        };
        tracing::info!(?action, %message, "候选右键菜单");
        self.refresh(client);
        host::with(|h| h.status = Some(message));
        self.render(client);
    }

    fn click(&self, target: HitTarget) {
        // SAFETY: IMKInputController 的 `client` 返回当前会话的文本客户端（实现 IMKTextInput 的代理），可能为 nil
        let client: Option<Retained<AnyObject>> = unsafe { msg_send![self, client] };
        let Some(client) = client else {
            return;
        };
        let client = TextClient::new(&client);
        let composing = host::with(|h| !h.engine.composition().is_empty()).unwrap_or(false);
        if !composing {
            return;
        }
        match target {
            HitTarget::Candidate(offset) => {
                if let Some(index) = host::with(|h| h.session.index_in_view(offset)).flatten() {
                    self.commit_index(index, client);
                }
            }
            HitTarget::Translation { candidate, sense } => {
                let index = host::with(|h| h.session.index_in_view(candidate)).flatten();
                self.commit_translation_at(index, sense, client);
            }
            // 菜单项在菜单自己的窗口里点，候选窗不会给出
            HitTarget::MenuItem(_) => {}
        }
    }
}
