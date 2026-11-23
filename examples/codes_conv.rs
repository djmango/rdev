use rdev::{usb_hid_code_to_linux_code, usb_hid_code_to_macos_code, usb_hid_code_to_win_scancode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Pass a hexadecimal HID keyboard usage, for example: cargo run --example codes_conv -- 0x14
    let arg = std::env::args().nth(1).unwrap_or_else(|| "0x14".into());
    let usage = u32::from_str_radix(arg.trim_start_matches("0x"), 16)?;
    println!(
        "HID {usage:#x}: Windows {:?}, X11 {:?}, macOS {:?}",
        usb_hid_code_to_win_scancode(usage),
        usb_hid_code_to_linux_code(usage),
        usb_hid_code_to_macos_code(usage)
    );
    Ok(())
}
