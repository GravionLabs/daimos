// The console window of a release build on Windows is not wanted.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    daimos_lib::run();
}
