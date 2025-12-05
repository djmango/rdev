use crate::rdev::DisplayError;
use objc2_core_graphics::CGMainDisplayID;

pub fn display_size() -> Result<(u64, u64), DisplayError> {
    let display_id = CGMainDisplayID();
    let width = objc2_core_graphics::CGDisplayPixelsWide(display_id);
    let height = objc2_core_graphics::CGDisplayPixelsHigh(display_id);
    Ok((width as u64, height as u64))
}
