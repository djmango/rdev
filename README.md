![](https://github.com/Narsil/rdev/workflows/build/badge.svg)
[![Crate](https://img.shields.io/crates/v/rdev.svg)](https://crates.io/crates/rdev)
[![API](https://docs.rs/rdev/badge.svg)](https://docs.rs/rdev)

# rdev

Simple library to listen and send events **globally** to keyboard and mouse on MacOS, Windows and Linux
(x11).

You can also check out [Enigo](https://github.com/Enigo-rs/Enigo) which is another
crate which helped me write this one.

This crate is so far a pet project for me to understand the rust ecosystem.

## Listening to global events

```rust
use rdev::{listen, Event};

// This will block.
if let Err(error) = listen(callback) {
    println!("Error: {:?}", error)
}

fn callback(event: Event) {
    if let rdev::EventType::ButtonPress(button) = event.event_type {
        println!("Mouse button pressed: {button:?}");
    }
}
```

## Threading Requirements

### macOS
The `listen()` and `grab()` functions **must** be called from the main thread. They use CoreFoundation's CFRunLoop which requires main thread execution. The process needs to be granted access to the Accessibility API (ie. if you're running your process inside Terminal.app, then Terminal.app needs to be added in System Preferences > Security & Privacy > Privacy > Accessibility).

**Note:** Unicode translation for keyboard events is now automatically handled via dispatch to the main thread, eliminating previous crashes.

### Windows  
The `listen()` and `grab()` functions should typically be called from the main thread or a dedicated thread with a message loop.

### Linux
The `listen()` and `grab()` functions can run on any thread. The `listen` function uses X11 APIs, and so will not work in Wayland or in the linux kernel virtual console.

### All Platforms
**Important:** Callbacks execute on OS-specific event threads, NOT the calling thread. For complex processing or communication with your main application, use channels (like `crossbeam-channel`) to send events to your application's processing thread.

Example with channels:
```rust
use rdev::listen;
use crossbeam_channel::unbounded;

let (tx, rx) = unbounded();
std::thread::spawn(move || {
    listen(move |event| {
        tx.send(event).unwrap_or_else(|e| eprintln!("Could not send event {:?}", e));
    }).expect("Could not listen");
});

for event in rx.iter() {
    println!("Received {:?}", event);
    // Process event in your application thread
}
```

## Sending some events

```rust
use rdev::{simulate, Button, EventType, Key, SimulateError};
use std::{thread, time};

fn send(event_type: &EventType) {
    let delay = time::Duration::from_millis(20);
    match simulate(event_type) {
        Ok(()) => (),
        Err(SimulateError) => {
            println!("We could not send {:?}", event_type);
        }
    }
    // Let ths OS catchup (at least MacOS)
    thread::sleep(delay);
}

send(&EventType::KeyPress(Key::KeyS));
send(&EventType::KeyRelease(Key::KeyS));

send(&EventType::MouseMove { x: 0.0, y: 0.0 });
send(&EventType::MouseMove { x: 400.0, y: 400.0 });
send(&EventType::ButtonPress(Button::Left));
send(&EventType::ButtonRelease(Button::Left));
send(&EventType::Wheel {
    delta_x: 0.0,
    delta_y: 1.0,
});
```
## Event types

An `Event` contains a timestamp, its `EventType`, and platform-specific metadata.
Match `event.event_type` for mouse buttons, motion, or wheel events. Wheel deltas use
floating-point line units. See `Event` and `EventType` in the API reference for all fields and variants.

## Getting the main screen size

```rust
use rdev::{display_size};

let (w, h) = display_size().unwrap();
assert!(w > 0);
assert!(h > 0);
```

## Keyboard state

We can define a dummy Keyboard, that we will use to detect
what kind of EventType trigger some String. We get the currently used
layout for now !
Caveat : This is layout dependent. If your app needs to support
layout switching don't use this !
Caveat: On Linux, the dead keys mechanism is not implemented.
Caveat: Only shift and dead keys are implemented, Alt+unicode code on windows
won't work.

```rust
use rdev::{Keyboard, EventType, Key, KeyboardState};

#[cfg(target_os = "windows")]
let mut keyboard = Keyboard::new();
#[cfg(not(target_os = "windows"))]
let mut keyboard = Keyboard::new().unwrap();
let string = keyboard.add(&EventType::KeyPress(Key::KeyS));
// string == Some("s")
```

## Platform support

This fork uses X11 on Linux, not Wayland or evdev. There is no `unstable_grab` Cargo feature.

## Serialization

Event data returned by the `listen` and `grab` functions can be serialized and de-serialized with
Serde if you install this library with the `serialize` feature.

