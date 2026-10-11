// Release builds start without a console; debug builds keep one for the log.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    lolping_lite::win::run();
    #[cfg(not(windows))]
    eprintln!("lolPing Lite runs on Windows.");
}
