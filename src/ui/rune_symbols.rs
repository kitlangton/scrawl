//! SF Symbols for the Rune windows, drawn by AppKit at 2x into `Pixels` so they match the
//! SwiftUI windows exactly. Rune draws only its own vector `Icon`s; each `Icon` here names its
//! SF Symbol and is the fallback when AppKit can't draw it.

use std::cell::RefCell;
use std::collections::HashMap;

use objc2::encode::{Encode, Encoding};
use objc2::rc::{Allocated, Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::msg_send;
use objc2_foundation::NSString;
use rune::{Color, El, Icon, Pixels, el, icon};

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Size {
    w: f64,
    h: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    origin: Point,
    size: Size,
}

unsafe impl Encode for Point {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for Size {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for Rect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[Point::ENCODING, Size::ENCODING]);
}

/// `NSFontWeight` values, as SwiftUI's `Font.Weight` maps to them.
pub(crate) mod weight {
    pub(crate) const REGULAR: f32 = 0.0;
    pub(crate) const MEDIUM: f32 = 0.23;
    pub(crate) const SEMIBOLD: f32 = 0.3;
    pub(crate) const BOLD: f32 = 0.4;
}

const SCALE: f64 = 2.0;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    name: &'static str,
    point: u32,
    weight: i32,
    color: [u8; 4],
}

thread_local! {
    static CACHE: RefCell<HashMap<Key, Option<(Pixels, f32, f32)>>> = RefCell::new(HashMap::new());
}

fn encode(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

/// `symbol`'s SF Symbol at `point` with `weight` (`weight::*`) in `color`, sized as SwiftUI's
/// `Image(systemName:).font(.system(size: point, weight:))` is.
pub(crate) fn symbol(symbol: &'static Icon, point: f32, weight: f32, color: Color) -> El {
    let key = Key {
        name: symbol.name,
        point: (point * 100.0).round() as u32,
        weight: (weight * 1000.0).round() as i32,
        color: [encode(color.r), encode(color.g), encode(color.b), (color.a.clamp(0.0, 1.0) * 255.0).round() as u8],
    };
    let drawn = CACHE.with(|cache| {
        cache
            .borrow_mut()
            .entry(key.clone())
            .or_insert_with(|| draw(symbol.name, point, weight, key.color))
            .clone()
    });
    match drawn {
        Some((pixels, w, h)) => el().size(w, h).pixels(&pixels),
        None => icon(symbol, point * 1.15).color(color),
    }
}

fn draw(name: &str, point: f32, weight: f32, color: [u8; 4]) -> Option<(Pixels, f32, f32)> {
    autoreleasepool(|_| unsafe {
        let image_class = AnyClass::get(c"NSImage")?;
        let config_class = AnyClass::get(c"NSImageSymbolConfiguration")?;
        let rep_class = AnyClass::get(c"NSBitmapImageRep")?;
        let context_class = AnyClass::get(c"NSGraphicsContext")?;

        let name = NSString::from_str(name);
        let image: *mut AnyObject = msg_send![
            image_class,
            imageWithSystemSymbolName: &*name,
            accessibilityDescription: std::ptr::null::<AnyObject>()
        ];
        if image.is_null() {
            return None;
        }
        let config: *mut AnyObject = msg_send![
            config_class,
            configurationWithPointSize: f64::from(point),
            weight: f64::from(weight),
            scale: 2_isize
        ];
        let image: *mut AnyObject = msg_send![image, imageWithSymbolConfiguration: config];
        if image.is_null() {
            return None;
        }
        let size: Size = msg_send![image, size];
        let (pw, ph) = ((size.w * SCALE).ceil() as usize, (size.h * SCALE).ceil() as usize);
        if pw == 0 || ph == 0 {
            return None;
        }
        let space = NSString::from_str("NSDeviceRGBColorSpace");
        let rep: Allocated<AnyObject> = msg_send![rep_class, alloc];
        let rep: Option<Retained<AnyObject>> = msg_send![
            rep,
            initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>(),
            pixelsWide: pw as isize,
            pixelsHigh: ph as isize,
            bitsPerSample: 8_isize,
            samplesPerPixel: 4_isize,
            hasAlpha: Bool::YES,
            isPlanar: Bool::NO,
            colorSpaceName: &*space,
            bytesPerRow: (pw * 4) as isize,
            bitsPerPixel: 32_isize
        ];
        let rep = rep?;
        let points = Size { w: pw as f64 / SCALE, h: ph as f64 / SCALE };
        let _: () = msg_send![&*rep, setSize: points];
        let context: *mut AnyObject = msg_send![context_class, graphicsContextWithBitmapImageRep: &*rep];
        if context.is_null() {
            return None;
        }
        let _: () = msg_send![context_class, saveGraphicsState];
        let _: () = msg_send![context_class, setCurrentContext: context];
        let rect = Rect { origin: Point::default(), size: points };
        let _: () = msg_send![
            image,
            drawInRect: rect,
            fromRect: Rect::default(),
            operation: 2_usize,
            fraction: 1.0_f64
        ];
        let _: () = msg_send![context, flushGraphics];
        let _: () = msg_send![context_class, restoreGraphicsState];

        let data: *const u8 = msg_send![&*rep, bitmapData];
        let row: isize = msg_send![&*rep, bytesPerRow];
        if data.is_null() {
            return None;
        }
        let mut rgba = Vec::with_capacity(pw * ph * 4);
        for y in 0..ph {
            for x in 0..pw {
                let alpha = *data.add(y * row as usize + x * 4 + 3);
                let a = (u16::from(alpha) * u16::from(color[3]) / 255) as u8;
                rgba.extend_from_slice(&[color[0], color[1], color[2], a]);
            }
        }
        let pixels = Pixels::new();
        pixels.set(pw as u32, ph as u32, rgba);
        Some((pixels, points.w as f32, points.h as f32))
    })
}
