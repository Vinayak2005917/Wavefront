use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc;
use std::thread;

pub const WM_HOTKEY: u32 = 0x0312;
pub const WM_QUIT: u32 = 0x0012;
pub const WM_APP_HOTKEY_UPDATE: u32 = 0x8001;
pub const PM_NOREMOVE: u32 = 0x0000;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_ALT: u32 = 0x0001;
pub const MOD_WIN: u32 = 0x0008;
pub const MOD_NOREPEAT: u32 = 0x4000;
pub const WH_KEYBOARD_LL: i32 = 13;
pub const WH_MOUSE_LL: i32 = 14;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_SYSKEYDOWN: u32 = 0x0104;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_MBUTTONDOWN: u32 = 0x0207;
pub const WM_XBUTTONDOWN: u32 = 0x020B;
pub const GWL_EXSTYLE: i32 = -20;
pub const WS_EX_TOOLWINDOW: isize = 0x00000080;
pub const WS_EX_NOACTIVATE: isize = 0x08000000;
pub const SW_SHOWNOACTIVATE: i32 = 4;
pub const SW_HIDE: i32 = 0;
pub const SWP_NOSIZE: u32 = 0x0001;
pub const SWP_NOMOVE: u32 = 0x0002;
pub const SWP_NOACTIVATE: u32 = 0x0010;
pub const SWP_FRAMECHANGED: u32 = 0x0020;
pub const HWND_TOPMOST: *mut c_void = -1isize as *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
pub struct Msg {
    pub hwnd: *mut c_void,
    pub message: u32,
    pub w_param: usize,
    pub l_param: isize,
    pub time: u32,
    pub point: Point,
    pub private: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyboardInput {
    pub vk: u16,
    pub scan: u16,
    pub flags: u32,
    pub time: u32,
    pub extra_info: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union InputData {
    pub keyboard: KeyboardInput,
    pub alignment: [usize; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Input {
    pub input_type: u32,
    pub padding: u32,
    pub data: InputData,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GuiThreadInfo {
    pub cb_size: u32,
    pub flags: u32,
    pub hwnd_active: *mut c_void,
    pub hwnd_focus: *mut c_void,
    pub hwnd_capture: *mut c_void,
    pub hwnd_menu_owner: *mut c_void,
    pub hwnd_move_size: *mut c_void,
    pub hwnd_caret: *mut c_void,
    pub rc_caret: Rect,
}
impl Default for GuiThreadInfo {
    fn default() -> Self {
        Self {
            cb_size: std::mem::size_of::<Self>() as u32,
            flags: 0,
            hwnd_active: std::ptr::null_mut(),
            hwnd_focus: std::ptr::null_mut(),
            hwnd_capture: std::ptr::null_mut(),
            hwnd_menu_owner: std::ptr::null_mut(),
            hwnd_move_size: std::ptr::null_mut(),
            hwnd_caret: std::ptr::null_mut(),
            rc_caret: Rect::default(),
        }
    }
}

#[repr(C)]
pub struct KbdLlHookStruct {
    pub vk_code: u32,
    pub scan_code: u32,
    pub flags: u32,
    pub time: u32,
    pub extra_info: usize,
}
#[repr(C)]
pub struct MouseLlHookStruct {
    pub point: Point,
    pub mouse_data: u32,
    pub flags: u32,
    pub time: u32,
    pub extra_info: usize,
}

#[repr(C)]
struct BrowseInfoW {
    owner: *mut c_void,
    root: *mut c_void,
    display_name: *mut u16,
    title: *const u16,
    flags: u32,
    callback: Option<unsafe extern "system" fn(*mut c_void, u32, isize, isize) -> i32>,
    parameter: isize,
    image: i32,
}

#[derive(Clone, Copy)]
pub struct FocusTarget {
    pub foreground: isize,
    pub thread_id: u32,
    pub process_id: u32,
    pub focused: isize,
}
impl FocusTarget {
    pub fn is_valid(&self) -> bool {
        self.foreground != 0 && self.thread_id != 0 && self.process_id != 0 && self.focused != 0
    }
}

static USER_INTERACTION: AtomicBool = AtomicBool::new(false);
static KEYBOARD_HOOK: AtomicIsize = AtomicIsize::new(0);
static MOUSE_HOOK: AtomicIsize = AtomicIsize::new(0);
const OWN_INPUT_MARKER: usize = 0x5746_524F_4E54;

pub fn user_interaction_detected() -> bool {
    USER_INTERACTION.load(Ordering::SeqCst)
}

pub fn capture_focus(foreground: isize) -> Option<FocusTarget> {
    if foreground == 0 {
        return None;
    }
    let hwnd = foreground as *mut c_void;
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, &mut process_id) };
    if thread_id == 0 || process_id == 0 {
        return None;
    }
    let mut info = GuiThreadInfo::default();
    if unsafe { GetGUIThreadInfo(thread_id, &mut info) } == 0 || info.hwnd_focus.is_null() {
        return None;
    }
    Some(FocusTarget {
        foreground,
        thread_id,
        process_id,
        focused: info.hwnd_focus as isize,
    })
}

pub fn pick_directory(title: &str) -> Option<String> {
    const BIF_RETURNONLYFSDIRS: u32 = 0x0001;
    const BIF_EDITBOX: u32 = 0x0010;
    const BIF_NEWDIALOGSTYLE: u32 = 0x0040;

    let com_initialized = unsafe { CoInitializeEx(std::ptr::null_mut(), 0x2) } >= 0;
    let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let mut display_name = [0u16; 260];
    let info = BrowseInfoW {
        owner: unsafe { GetForegroundWindow() },
        root: std::ptr::null_mut(),
        display_name: display_name.as_mut_ptr(),
        title: title.as_ptr(),
        flags: BIF_RETURNONLYFSDIRS
            | if com_initialized {
                BIF_EDITBOX | BIF_NEWDIALOGSTYLE
            } else {
                0
            },
        callback: None,
        parameter: 0,
        image: 0,
    };
    let item = unsafe { SHBrowseForFolderW(&info) };
    let selected = if item.is_null() {
        None
    } else {
        let mut path = [0u16; 32_768];
        let succeeded = unsafe { SHGetPathFromIDListW(item, path.as_mut_ptr()) } != 0;
        unsafe { CoTaskMemFree(item) };
        if succeeded {
            let length = path.iter().position(|unit| *unit == 0).unwrap_or(path.len());
            Some(String::from_utf16_lossy(&path[..length]))
        } else {
            None
        }
    };
    if com_initialized {
        unsafe { CoUninitialize() };
    }
    selected
}

/// Verify both the foreground window and the exact focused control immediately before edits.
pub unsafe fn same_focus(target: &FocusTarget) -> bool {
    if unsafe { GetForegroundWindow() } as isize != target.foreground {
        return false;
    }
    let mut process_id = 0;
    if unsafe { GetWindowThreadProcessId(target.foreground as *mut c_void, &mut process_id) }
        != target.thread_id
        || process_id != target.process_id
    {
        return false;
    }
    let mut info = GuiThreadInfo::default();
    (unsafe { GetGUIThreadInfo(target.thread_id, &mut info) }) != 0
        && info.hwnd_focus as isize == target.focused
}

pub struct InteractionHooks {
    thread_id: u32,
    stop: mpsc::Sender<()>,
}
impl InteractionHooks {
    pub fn start() -> Result<Self, String> {
        USER_INTERACTION.store(false, Ordering::SeqCst);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (stop_tx, stop_rx) = mpsc::channel();
        let _ = thread::Builder::new()
            .name("wavefront-input-guard".into())
            .spawn(move || {
                let thread_id = unsafe { GetCurrentThreadId() };
                let module = unsafe { GetModuleHandleW(std::ptr::null()) };
                let keyboard = unsafe {
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard), module, 0)
                };
                let mouse =
                    unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse), module, 0) };
                KEYBOARD_HOOK.store(keyboard as isize, Ordering::SeqCst);
                MOUSE_HOOK.store(mouse as isize, Ordering::SeqCst);
                if keyboard.is_null() || mouse.is_null() {
                    let _ = ready_tx.send(Err(format!(
                        "Could not install Windows input safety hooks (error {}).",
                        unsafe { GetLastError() }
                    )));
                } else {
                    let _ = ready_tx.send(Ok(thread_id));
                }
                if keyboard.is_null() || mouse.is_null() {
                    unsafe {
                        if !keyboard.is_null() {
                            UnhookWindowsHookEx(keyboard);
                        }
                        if !mouse.is_null() {
                            UnhookWindowsHookEx(mouse);
                        }
                    }
                    return;
                }
                // Hook callbacks are dispatched on this thread, so it must pump Windows messages.
                loop {
                    let mut msg = Msg {
                        hwnd: std::ptr::null_mut(),
                        message: 0,
                        w_param: 0,
                        l_param: 0,
                        time: 0,
                        point: Point::default(),
                        private: 0,
                    };
                    if unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } <= 0 {
                        break;
                    }
                    unsafe {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }
                }
                unsafe {
                    UnhookWindowsHookEx(keyboard);
                    UnhookWindowsHookEx(mouse);
                }
                KEYBOARD_HOOK.store(0, Ordering::SeqCst);
                MOUSE_HOOK.store(0, Ordering::SeqCst);
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                stop: stop_tx,
            }),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(format!(
                "Timed out installing Windows input safety hooks: {e}"
            )),
        }
    }
}
impl Drop for InteractionHooks {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        unsafe {
            PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
        }
    }
}

unsafe extern "system" fn low_level_keyboard(code: i32, message: usize, data: isize) -> isize {
    if code >= 0 && (message as u32 == WM_KEYDOWN || message as u32 == WM_SYSKEYDOWN) {
        let event = unsafe { &*(data as *const KbdLlHookStruct) };
        if event.extra_info != OWN_INPUT_MARKER {
            USER_INTERACTION.store(true, Ordering::SeqCst);
        }
    }
    let hook = KEYBOARD_HOOK.load(Ordering::SeqCst) as *mut c_void;
    unsafe { CallNextHookEx(hook, code, message, data) }
}
unsafe extern "system" fn low_level_mouse(code: i32, message: usize, data: isize) -> isize {
    if code >= 0
        && matches!(
            message as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
    {
        let event = unsafe { &*(data as *const MouseLlHookStruct) };
        if event.extra_info != OWN_INPUT_MARKER {
            USER_INTERACTION.store(true, Ordering::SeqCst);
        }
    }
    let hook = MOUSE_HOOK.load(Ordering::SeqCst) as *mut c_void;
    unsafe { CallNextHookEx(hook, code, message, data) }
}

#[link(name = "user32")]
unsafe extern "system" {
    pub fn RegisterHotKey(hwnd: *mut c_void, id: i32, modifiers: u32, key: u32) -> i32;
    pub fn UnregisterHotKey(hwnd: *mut c_void, id: i32) -> i32;
    pub fn GetMessageW(msg: *mut Msg, hwnd: *mut c_void, min: u32, max: u32) -> i32;
    pub fn PeekMessageW(msg: *mut Msg, hwnd: *mut c_void, min: u32, max: u32, remove: u32) -> i32;
    pub fn TranslateMessage(msg: *const Msg) -> i32;
    pub fn DispatchMessageW(msg: *const Msg) -> isize;
    pub fn PostThreadMessageW(thread_id: u32, message: u32, w_param: usize, l_param: isize) -> i32;
    pub fn GetForegroundWindow() -> *mut c_void;
    pub fn GetWindowThreadProcessId(hwnd: *mut c_void, process_id: *mut u32) -> u32;
    pub fn GetGUIThreadInfo(thread_id: u32, info: *mut GuiThreadInfo) -> i32;
    pub fn SetWindowsHookExW(
        id: i32,
        proc: Option<unsafe extern "system" fn(i32, usize, isize) -> isize>,
        module: *mut c_void,
        thread_id: u32,
    ) -> *mut c_void;
    pub fn UnhookWindowsHookEx(hook: *mut c_void) -> i32;
    pub fn CallNextHookEx(hook: *mut c_void, code: i32, w_param: usize, l_param: isize) -> isize;
    pub fn SetWindowLongPtrW(hwnd: *mut c_void, index: i32, value: isize) -> isize;
    pub fn GetWindowLongPtrW(hwnd: *mut c_void, index: i32) -> isize;
    pub fn SetWindowPos(
        hwnd: *mut c_void,
        insert_after: *mut c_void,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
    pub fn ShowWindow(hwnd: *mut c_void, command: i32) -> i32;
    pub fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn GetCurrentThreadId() -> u32;
    pub fn GetLastError() -> u32;
    pub fn GetModuleHandleW(name: *const u16) -> *mut c_void;
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHBrowseForFolderW(info: *const BrowseInfoW) -> *mut c_void;
    fn SHGetPathFromIDListW(item: *const c_void, path: *mut u16) -> i32;
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, concurrency: u32) -> i32;
    fn CoUninitialize();
    fn CoTaskMemFree(memory: *mut c_void);
}
