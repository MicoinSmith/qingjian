//! 单击 Shift 切换中 / 英（配置 `[shortcut] switch_mode` 勾了 shift 时）。
//!
//! IMK 只把普通按键送进 `handleEvent:`，修饰键的按下 / 抬起拿不到，所以用定时器轮询
//! `NSEvent::modifierFlags_class()` 做边沿检测：Shift 按下到抬起之间没有别的键插进来就是一次单击。
//! 规则与 Windows 的 `com/key/tap.rs` 相同，但那边能看到被系统热键吃掉的键并作废这次单击，macOS 看不到
//! （系统热键不进 `handleEvent:`），所以「按住 Shift 按了系统快捷键再松开」会被误判成一次单击。
//! 勾上之后中英由本模块决定，Caps Lock 退化成只管大小写。

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSObject, NSObjectProtocol, NSTimer};

use crate::imk::modifiers;

use super::Host;

/// 轮询 Shift 状态的间隔。用户点一下 Shift 大约百来毫秒，30 ms 够采到按下那一帧；再密下去白耗电。
const POLL_INTERVAL: f64 = 0.03;

/// 左右两个 Shift 的键码。只有 Shift 自己不算「插进来的别的键」：按下它本来就是这次单击的一部分；
/// 别的修饰键（Cmd / Ctrl / Option / Caps Lock）按下一律作废，与 Windows 的 `tap_key` 同口径。
fn is_shift_key(key_code: u16) -> bool {
    matches!(key_code, 56 | 60)
}

/// 单击 Shift 的判定与它切出来的持久英文模式。
pub struct ModeSwitch {
    /// 输入法正激活（在某个应用的输入框里）；定时器只在激活且勾了 Shift 时跑。
    active: bool,

    /// 配置里勾了「单击 Shift」（`[shortcut] switch_mode` 含 shift）。
    enabled: bool,

    /// 上一次轮询时 Shift 是否按着，用来认边沿。
    pressed: bool,

    /// Shift 按下之后还没有别的键插进来，抬起就算一次单击。
    pending: bool,

    /// 单击 Shift 切出来的持久英文模式；`enabled` 为假时无意义（`set_enabled(false)` 会清零）。
    english: bool,

    timer: Option<Retained<NSTimer>>,

    mtm: MainThreadMarker,
}

impl ModeSwitch {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self {
            active: false,
            enabled: false,
            pressed: false,
            pending: false,
            english: false,
            timer: None,
            mtm,
        }
    }

    /// 配置里勾了「单击 Shift」。
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// 单击 Shift 切出来的英文模式。
    pub fn english(&self) -> bool {
        self.english
    }

    /// 配置里的开关变了。
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        // 关掉时退回中文，免得留着上次切出来的英文模式
        if !enabled {
            self.english = false;
        }
        self.sync();
    }

    /// 输入法激活 / 停用（定时器只在激活期间跑）。
    pub fn set_active(&mut self, active: bool) {
        if self.active == active {
            return;
        }
        self.active = active;
        // 焦点切换不算一次单击：按当前硬件状态重新对齐，免得「按着 Shift 切走再切回、然后松开」被当成单击
        self.pressed = modifiers::shift_down();
        self.pending = false;
        self.sync();
    }

    /// 任一普通按键按下：Shift 按下到抬起之间插了别的键，这次不算单击。
    pub fn note_key(&mut self) {
        self.pending = false;
    }

    /// 轮询一次；这次轮询切换了中英返回 `true`。
    fn poll(&mut self) -> bool {
        let down = modifiers::shift_down();
        if down == self.pressed {
            return false;
        }
        self.pressed = down;
        if down {
            self.pending = true;
            return false;
        }
        if !std::mem::take(&mut self.pending) {
            return false;
        }
        self.english = !self.english;
        true
    }

    /// 按「激活且勾了 Shift」决定定时器开还是关。
    fn sync(&mut self) {
        let wanted = self.active && self.enabled;
        if wanted == self.timer.is_some() {
            return;
        }
        if wanted {
            self.timer = Some(self.schedule());
        } else if let Some(timer) = self.timer.take() {
            timer.invalidate();
        }
    }

    fn schedule(&self) -> Retained<NSTimer> {
        let target = ModeSwitchTick::new(self.mtm);
        unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                POLL_INTERVAL,
                &target,
                sel!(tick:),
                None,
                true,
            )
        }
    }
}

impl Host {
    /// 中英模式：勾了「单击 Shift」时由 Shift 的切换决定（Caps Lock 退化成只管大小写），没勾看 Caps Lock。
    pub fn english_mode(&self) -> bool {
        if self.mode_switch.enabled() {
            self.mode_switch.english()
        } else {
            modifiers::caps_lock_on()
        }
    }

    /// 英文模式下一个字母要不要大写：勾了「单击 Shift」时 Caps Lock 只管大小写（与 Shift 异或）；
    /// 没勾时 Caps Lock 就是中英开关，不参与大小写。
    pub fn english_letter_uppercase(&self) -> bool {
        if self.mode_switch.enabled() {
            modifiers::shift_down() != modifiers::caps_lock_on()
        } else {
            modifiers::shift_down()
        }
    }

    /// 轮询到一次单击 Shift：切换中英并刷新界面；这次真的切换了返回 `true`。
    pub fn poll_mode_switch(&mut self) -> bool {
        if !self.mode_switch.poll() {
            return false;
        }
        tracing::info!(english = self.english_mode(), "单击 Shift 切换中英");
        self.refresh_mode_ui();
        self.render();
        true
    }

    /// 中英模式或 Caps Lock 可能变了：刷新状态条与候选窗口的输入状态，变了就弹一次右上角提示。
    ///
    /// 中英变了显示「中」/「英」（优先）；只有大小写变了才显示「大写」/「小写」——后者出现在勾了
    /// 「单击 Shift」之后，那时 Caps Lock 退化成只管大小写。
    pub fn refresh_mode_ui(&mut self) {
        let english = self.english_mode();
        self.indicator.update(english);
        let caps = modifiers::caps_lock_on();
        // 第一次只记不弹：启动或刚激活时不该凭空冒一个提示
        if self.last_osd_english.is_some_and(|last| last != english) {
            self.osd.show(if english { "英" } else { "中" });
        } else if self.last_osd_caps.is_some_and(|last| last != caps) {
            self.osd.show(if caps { "大写" } else { "小写" });
        }
        self.last_osd_english = Some(english);
        self.last_osd_caps = Some(caps);
    }

    /// 停用时忘掉已提示过的状态：下次激活的第一次刷新只记不弹，焦点来回挪不至于弹提示。
    pub fn forget_osd_state(&mut self) {
        self.last_osd_english = None;
        self.last_osd_caps = None;
    }

    /// 任一按键按下：除 Shift 自己以外的任何键（含别的修饰键）都作废待定的单击 Shift
    /// （Shift + 字母 / Shift + Cmd 组合都不该被当成单击）。
    pub fn note_key(&mut self, key_code: u16) {
        if !is_shift_key(key_code) {
            self.mode_switch.note_key();
        }
    }
}

define_class!(
    // SAFETY: NSObject 没有子类化要求；没有实现 Drop。
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct ModeSwitchTick;

    impl ModeSwitchTick {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: Option<&AnyObject>) {
            // 定时器回调也是 ObjC 运行时直接调的，panic 同样不能穿出去
            match crate::imk::catch_panic("单击 Shift 轮询", || {
                crate::host::with(Host::poll_mode_switch)
            }) {
                // 切换了：组句中的拼音原样当英文上屏。要借文本客户端，所以放在 Host 借用之外
                Some(Some(true)) => {
                    if crate::imk::catch_panic("切换中英后落字", crate::imk::commit_raw_after_switch)
                        .is_none()
                    {
                        crate::imk::recover_from_panic(None);
                    }
                }
                // 没切换、或 Host 未初始化 / 正在被借用
                Some(_) => {}
                // panic 穿出来了，走既有善后
                None => crate::imk::recover_from_panic(None),
            }
        }
    }

    unsafe impl NSObjectProtocol for ModeSwitchTick {}
);

impl ModeSwitchTick {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}
