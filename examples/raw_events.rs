//! Test Raw Input events - run on Windows to verify touchpad/keyboard capture
//!
//! Usage:
//!   cargo build --example raw_events --target x86_64-pc-windows-gnu
//!   # Copy raw_events.exe to Windows and run it
//!
//! This will print:
//!   [RAW] events from Raw Input API
//!   [HOOK] events from low-level hooks

use rdev::{Event, EventType, listen};

fn main() {
    println!("=== Raw Input Debug Test ===");
    println!("Listening for ALL events (including raw)...");
    println!("Move mouse, scroll, type keys, use touchpad");
    println!("Press Ctrl+C to exit\n");

    if let Err(e) = listen(|event: Event| {
        match event.event_type {
            // Raw events (from Raw Input API on Windows, CGEventTap on macOS)
            EventType::MouseMoveRaw { delta_x, delta_y } => {
                println!("[RAW] MouseMove: dx={}, dy={}", delta_x, delta_y);
            }
            EventType::WheelRaw { delta_x, delta_y } => {
                println!("[RAW] Wheel: dx={:.2}, dy={:.2}", delta_x, delta_y);
            }
            EventType::KeyPressRaw(key) => {
                println!("[RAW] KeyPress: {:?}", key);
            }
            EventType::KeyReleaseRaw(key) => {
                println!("[RAW] KeyRelease: {:?}", key);
            }
            EventType::ButtonPressRaw(btn) => {
                println!("[RAW] ButtonPress: {:?}", btn);
            }
            EventType::ButtonReleaseRaw(btn) => {
                println!("[RAW] ButtonRelease: {:?}", btn);
            }

            // Regular events (from hooks on Windows, CGEventTap on macOS)
            EventType::Wheel { delta_x, delta_y } => {
                println!("[HOOK] Wheel: dx={:.2}, dy={:.2}", delta_x, delta_y);
            }
            EventType::KeyPress(key) => {
                println!("[HOOK] KeyPress: {:?}", key);
            }
            EventType::KeyRelease(key) => {
                println!("[HOOK] KeyRelease: {:?}", key);
            }
            EventType::ButtonPress(btn) => {
                println!("[HOOK] ButtonPress: {:?}", btn);
            }
            EventType::ButtonRelease(btn) => {
                println!("[HOOK] ButtonRelease: {:?}", btn);
            }
            EventType::MouseMove { x, y } => {
                // Skip verbose mouse move from hooks (too noisy)
                // Uncomment to see: println!("[HOOK] MouseMove: x={:.0}, y={:.0}", x, y);
                let _ = (x, y);
            }
        }
    }) {
        eprintln!("Error: {:?}", e);
    }
}
