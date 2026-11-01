use std::ffi::c_void;

pub const WM_HOTKEY: u32 = 0x0312;
pub const WM_QUIT: u32 = 0x0012;
pub const WM_APP_HOTKEY_UPDATE: u32 = 0x8001;
pub const PM_NOREMOVE: u32 = 0x0000;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_ALT: u32 = 0x0001;
pub const MOD_WIN: u32 = 0x0008;
pub const MOD_NOREPEAT: u32 = 0x4000;
pub const VK_D: u32 = 0x44;
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
}
