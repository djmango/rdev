use rdev::{EventType, listen};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    let (sender, mut receiver) = mpsc::channel(32);
    tokio::spawn(async move {
        while let Some(button) = receiver.recv().await {
            println!("Mouse button pressed: {button:?}");
        }
    });

    // Keep the native event loop on the main thread. Don't block it on a slow reader.
    listen(move |event| {
        if let EventType::ButtonPress(button) = event.event_type {
            let _ = sender.try_send(button);
        }
    })
    .expect("Could not listen");
}
