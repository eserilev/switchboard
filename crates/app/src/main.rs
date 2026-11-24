//! The desktop entry point: the app lives in the lib crate, so a phone build can share it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    switchboard_lib::start()
}
