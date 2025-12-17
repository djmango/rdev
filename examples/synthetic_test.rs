//! Test that simulated events are correctly detected as synthetic.
//!
//! This example:
//! 1. Starts a listener in a background thread
//! 2. Simulates keyboard and mouse events
//! 3. Verifies that received events have `is_synthetic: true`
//!
//! Run with: cargo run --example synthetic_test

use rdev::{listen, simulate, Event, EventType, Key};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn main() {
    println!("=== is_synthetic Detection Test ===\n");

    let listener_ready = Arc::new(AtomicBool::new(false));
    let listener_ready_clone = listener_ready.clone();

    let synthetic_count = Arc::new(AtomicUsize::new(0));
    let non_synthetic_count = Arc::new(AtomicUsize::new(0));
    let synthetic_count_clone = synthetic_count.clone();
    let non_synthetic_count_clone = non_synthetic_count.clone();

    // Start listener in background thread
    let _listener = thread::spawn(move || {
        listener_ready_clone.store(true, Ordering::SeqCst);

        if let Err(e) = listen(move |event: Event| {
            let synthetic_str = if event.is_synthetic {
                synthetic_count_clone.fetch_add(1, Ordering::SeqCst);
                "SYNTHETIC"
            } else {
                non_synthetic_count_clone.fetch_add(1, Ordering::SeqCst);
                "HARDWARE"
            };

            // Only print keyboard events to reduce noise
            match event.event_type {
                EventType::KeyPress(_) | EventType::KeyRelease(_) => {
                    println!("[{}] {:?}", synthetic_str, event.event_type);
                }
                EventType::KeyPressRaw(_) | EventType::KeyReleaseRaw(_) => {
                    println!("[{}] {:?}", synthetic_str, event.event_type);
                }
                _ => {}
            }
        }) {
            eprintln!("Listen error: {:?}", e);
        }
    });

    // Wait for listener to be ready
    while !listener_ready.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(10));
    }
    // Give the event tap time to fully initialize
    thread::sleep(Duration::from_millis(500));

    println!("Listener started. Simulating events...\n");

    // Simulate some keyboard events
    let test_keys = [Key::KeyA, Key::KeyB, Key::KeyC];

    for key in &test_keys {
        println!("Simulating press and release of {:?}", key);

        if let Err(e) = simulate(&EventType::KeyPress(*key)) {
            eprintln!("Failed to simulate key press: {:?}", e);
        }
        thread::sleep(Duration::from_millis(50));

        if let Err(e) = simulate(&EventType::KeyRelease(*key)) {
            eprintln!("Failed to simulate key release: {:?}", e);
        }
        thread::sleep(Duration::from_millis(100));
    }

    // Wait for events to be processed
    thread::sleep(Duration::from_millis(500));

    println!("\n=== Results ===");
    println!(
        "Synthetic events received: {}",
        synthetic_count.load(Ordering::SeqCst)
    );
    println!(
        "Hardware events received: {}",
        non_synthetic_count.load(Ordering::SeqCst)
    );

    let synthetic = synthetic_count.load(Ordering::SeqCst);
    if synthetic > 0 {
        println!("\n✓ SUCCESS: Simulated events were correctly detected as synthetic!");
    } else {
        println!("\n✗ FAILURE: No synthetic events detected. is_synthetic detection may be broken.");
    }

    println!("\nNote: You may also see hardware events if you pressed keys during the test.");
    println!("Press Ctrl+C to exit.");

    // Keep running so user can also test with real keyboard input
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

