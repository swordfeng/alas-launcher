////// Utility: hide console windows at start (Windows)
#[cfg(windows)]
use command_group::builder::CommandGroupBuilder;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
static HAS_CONSOLE: AtomicBool = AtomicBool::new(false);

pub trait CreateNoWindow {
    fn create_no_window(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl CreateNoWindow for Command {
    fn create_no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        use winapi::um::winbase::CREATE_NO_WINDOW;
        if !HAS_CONSOLE.load(Ordering::Relaxed) {
            self.creation_flags(CREATE_NO_WINDOW)
        } else {
            self
        }
    }
}

#[cfg(windows)]
impl<T> CreateNoWindow for CommandGroupBuilder<'_, T> {
    fn create_no_window(&mut self) -> &mut Self {
        use winapi::um::winbase::CREATE_NO_WINDOW;
        if !HAS_CONSOLE.load(Ordering::Relaxed) {
            self.creation_flags(CREATE_NO_WINDOW)
        } else {
            self
        }
    }
}

#[cfg(not(windows))]
impl<T> CreateNoWindow for T {
    fn create_no_window(&mut self) -> &mut Self {
        self
    }
}
