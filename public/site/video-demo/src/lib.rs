use fframes::{
    AudioMap, AudioTimestamp::*, AudioTrack, Color, Duration, FFramesContext, Frame, Overlap,
    Scene, Scenes, Svgr, Transform, Video, animation::Easing, include_media_dir,
};

include_media_dir!(pub struct ApparentDemoMedia, "media");

pub const WIDTH: usize = 1920;
pub const HEIGHT: usize = 1080;

const BG_DARK: &str = "#0b0c0f";
const SURFACE: &str = "#14161b";
const CARD_BG: &str = "#191c23";
const PILL_BG: &str = "#141416";
const LINE_SUBTLE: &str = "#272b35";
const TEXT_WHITE: &str = "#f4f5f8";
const TEXT_SOFT: &str = "#c6cad4";
const TEXT_MUTED: &str = "#828897";
const ACCENT_RED: &str = "#ff3b30";
const ACCENT_BLUE: &str = "#4b73ff";
const ACCENT_GREEN: &str = "#5cd696";

const FONT_SANS: &str = "DM Sans";
const FONT_DISPLAY: &str = "Inter 24pt";
const FONT_MONO: &str = "JetBrains Mono";

pub struct ApparentDemoVideo<'a> {
    pub media: &'a ApparentDemoMedia,
    intro: IntroScene<'a>,
    target: TargetScene,
    draw: DrawScene,
    bundle: BundleScene<'a>,
    paste: PasteScene,
}

impl<'a> ApparentDemoVideo<'a> {
    pub fn new(media: &'a ApparentDemoMedia, _title: &'a str) -> Self {
        Self {
            media,
            intro: IntroScene { media },
            target: TargetScene,
            draw: DrawScene,
            bundle: BundleScene { media },
            paste: PasteScene,
        }
    }
}

impl std::fmt::Debug for ApparentDemoVideo<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApparentDemoVideo").finish()
    }
}

impl Video for ApparentDemoVideo<'_> {
    const FPS: usize = 60;
    const WIDTH: usize = WIDTH;
    const HEIGHT: usize = HEIGHT;
    const BACKGROUND_COLOR: Color = Color::hex(BG_DARK);

    fn duration(&self) -> Duration<'_> {
        Duration::Auto
    }

    fn audio(&self) -> AudioMap<'_> {
        AudioMap::from([AudioTrack::new("soundtrack.wav", Second(0.0)..Eof)
            .gain_db(-2.0)
            .fade_in(0.25)
            .fade_out(0.8)])
    }

    fn define_scenes(&self) -> Scenes<'_> {
        Scenes::from(vec![
            &self.intro as &dyn Scene,
            &self.target,
            &self.draw,
            &self.bundle,
            &self.paste,
        ])
    }

    fn render_frame<'a>(&'a self, frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let total_sec = 23.0_f32;
        let progress_w = ((frame.seconds() / total_sec).clamp(0.0, 1.0) * 1600.0).max(1.0);
        fframes::svgr!(
            <svg xmlns="http://www.w3.org/2000/svg" width={WIDTH} height={HEIGHT} viewBox="0 0 1920 1080">
                <defs>
                    <radialGradient id="bg-glow" cx="50%" cy="22%" r="65%">
                        <stop offset="0%" stop-color="#1e2436" stop-opacity="0.75" />
                        <stop offset="100%" stop-color="#0b0c0f" stop-opacity="1.0" />
                    </radialGradient>
                    <linearGradient id="card-sheen" x1="0%" y1="0%" x2="100%" y2="100%">
                        <stop offset="0%" stop-color="#ffffff" stop-opacity="0.09" />
                        <stop offset="100%" stop-color="#ffffff" stop-opacity="0.01" />
                    </linearGradient>
                </defs>
                <rect x="0" y="0" width="1920" height="1080" fill="url(#bg-glow)" />
                {ctx.render_scenes(&frame)}
                // Subtle bottom timeline progress bar
                <rect x="160" y="1024" width="1600" height="3" rx="1.5" fill="#1e222b" />
                <rect x="160" y="1024" width={progress_w} height="3" rx="1.5" fill={ACCENT_RED} />
            </svg>
        )
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(0.0001)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn smootherstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(0.0001)).clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn render_keycap(x: f32, y: f32, w: f32, label: &str, pressed: f32) -> Svgr<'static> {
    let dy = pressed * 4.0;
    let border_col = if pressed > 0.5 { "#ff6b63" } else { "#363b47" };
    let bg_col = if pressed > 0.5 { "#2b191b" } else { "#1a1d24" };
    let text_x = x + w * 0.5;
    let text_y = y + dy + 42.0;
    fframes::svgr!(
        <g>
            <rect x={x} y={y + 6.0} width={w} height="64" rx="14" fill="#08090c" />
            <rect x={x} y={y + dy} width={w} height="64" rx="14" fill={bg_col} stroke={border_col} stroke-width="2" />
            <text x={text_x} y={text_y} text-anchor="middle" font-family={FONT_MONO} font-weight="400" font-size="28" fill={TEXT_WHITE}>
                {label.to_owned()}
            </text>
        </g>
    )
}

/// Scene 1 (0..4.4s): Hero Liquid Glass Icon + Dynamic Island HUD + ⌃⌥⇧S Trigger
pub struct IntroScene<'a> {
    media: &'a ApparentDemoMedia,
}

impl std::fmt::Debug for IntroScene<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntroScene").finish()
    }
}

impl Scene for IntroScene<'_> {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(4.4)
    }

    fn render_frame<'a>(&'a self, frame: Frame, _ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.seconds();
        let exit_op = 1.0 - smoothstep(4.0, 4.38, t);
        let press = (smoothstep(1.4, 1.6, t) * (1.0 - smoothstep(1.85, 2.05, t))).clamp(0.0, 1.0);
        let pill_scale: f32 = frame.animate(&fframes::timeline!(
            at 1.55, animate 0.55_f32 => 1.0_f32,
                Easing::Spring { mass: 1.0, stiffness: 240.0, damping: 19.0 },
        ));
        let pill_op = smoothstep(1.55, 1.78, t);
        let pill_tx = 960.0 * (1.0 - pill_scale);
        let pill_ty = 76.0 * (1.0 - pill_scale);

        let keys: Vec<Svgr<'static>> = [
            (690.0, 88.0, "⌃"),
            (792.0, 88.0, "⌥"),
            (894.0, 88.0, "⇧"),
            (996.0, 134.0, "S"),
        ]
        .into_iter()
        .map(|(kx, kw, label)| render_keycap(kx, 825.0, kw, label, press))
        .collect();

        fframes::svgr!(
            <g opacity={exit_op}>
                // Dynamic Island Pill blooming open at top-center after ⌃⌥⇧S press
                <g opacity={pill_op} transform={Transform {
                    translate_x: f64::from(pill_tx),
                    translate_y: f64::from(pill_ty),
                    scale: fframes::Scale {
                        x: f64::from(pill_scale),
                        y: f64::from(pill_scale),
                    },
                    ..Default::default()
                }}>
                    <rect x="770" y="52" width="380" height="52" rx="26" fill={PILL_BG} stroke="#343844" stroke-width="1.5" />
                    <rect x="780" y="60" width="96" height="36" rx="18" fill="#ffffff" fill-opacity="0.16" />
                    <text x="828" y="84" text-anchor="middle" font-family={FONT_SANS} font-weight="500" font-size="20" fill={TEXT_WHITE}>"Area"</text>
                    <text x="955" y="84" text-anchor="middle" font-family={FONT_SANS} font-weight="500" font-size="20" fill={TEXT_SOFT}>"Full Screen"</text>
                    <rect x="1038" y="66" width="1.5" height="24" fill="#2e323d" />
                    <text x="1092" y="85" text-anchor="middle" font-family={FONT_MONO} font-weight="400" font-size="20" fill={TEXT_MUTED}>"x"</text>
                </g>

                // Hero Liquid Glass App Icon (visible from frame 0, settles smoothly)
                <g transform={frame.animate(fframes::timeline!(
                    at 0.0, animate Transform::translate(0, 32) => Transform::translate(0, 0),
                        Easing::Spring { mass: 1.0, stiffness: 180.0, damping: 18.0 },
                ))}
                opacity={frame.animate(fframes::timeline!(
                    at 0.0 => 0.28, animate 0.65 => 1.0, Easing::EaseOut,
                ))}>
                    <image href={self.media.app_icon_png.href()} x="820" y="170" width="280" height="280" />
                </g>

                // Title & subtitle
                <g transform={frame.animate(fframes::timeline!(
                    at 0.05, animate Transform::translate(0, 28) => Transform::translate(0, 0),
                        Easing::Spring { mass: 1.0, stiffness: 180.0, damping: 20.0 },
                ))}
                opacity={frame.animate(fframes::timeline!(
                    at 0.0 => 0.35, animate 0.55 => 1.0, Easing::EaseOut,
                ))}>
                    <text x="960" y="545" text-anchor="middle" font-family={FONT_DISPLAY} font-weight="700" font-size="104" letter-spacing="-2" fill={TEXT_WHITE}>
                        "Apparent"
                    </text>
                    <text x="960" y="622" text-anchor="middle" font-family={FONT_SANS} font-weight="500" font-size="42" fill={TEXT_SOFT}>
                        "Screen, voice, and freehand ink for coding agents"
                    </text>
                </g>

                // Shortcut callout
                <g opacity={smoothstep(0.45, 0.85, t)}>
                    <text x="960" y="785" text-anchor="middle" font-family={FONT_MONO} font-weight="400" font-size="28" fill={TEXT_MUTED}>
                        "PRESS SHORTCUT ANYWHERE TO START"
                    </text>
                    {keys}
                </g>
            </g>
        )
    }
}

/// Scene 2 (4.4..9.4s): Area Drag Cutout -> Space Camera Cursor -> Window Highlight
#[derive(Debug)]
pub struct TargetScene;

impl Scene for TargetScene {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(5.0)
    }

    fn overlap(&self) -> Overlap {
        Overlap::Previous(0.3)
    }

    fn render_frame<'a>(&'a self, frame: Frame, _ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.seconds();
        let enter_op = smoothstep(0.0, 0.25, t) * (1.0 - smoothstep(4.65, 4.98, t));
        let in_window_mode = t >= 2.35;

        // Area drag rectangle grows from (420, 290) to (1020, 740) during 0.3..1.7s
        let drag_p = smootherstep(0.35, 1.65, t);
        let area_w = (40.0 + 620.0 * drag_p).max(2.0);
        let area_h = (30.0 + 430.0 * drag_p).max(2.0);

        // Window highlight glides onto the right-hand card when Space is pressed at t=2.35s
        let win_glide = smootherstep(2.35, 2.85, t);
        let sel_x = if in_window_mode {
            420.0 + (820.0 - 420.0) * win_glide
        } else {
            420.0
        };
        let sel_y = if in_window_mode {
            270.0 + (230.0 - 270.0) * win_glide
        } else {
            270.0
        };
        let sel_w = if in_window_mode {
            660.0 + (780.0 - 660.0) * win_glide
        } else {
            area_w
        };
        let sel_h = if in_window_mode {
            460.0 + (540.0 - 460.0) * win_glide
        } else {
            area_h
        };
        let sel_rx = if in_window_mode { 16.0 } else { 3.0 };
        let sel_stroke = if in_window_mode { ACCENT_BLUE } else { TEXT_WHITE };
        let sel_fill_op = if in_window_mode { 0.14 } else { 0.0 };

        let space_press = (smoothstep(2.2, 2.35, t) * (1.0 - smoothstep(2.6, 2.8, t))).clamp(0.0, 1.0);
        let step_title = if in_window_mode {
            "1 · Hit Space for Camera Window Picker"
        } else {
            "1 · Drag an Area or Hit Space for Window"
        };

        let cam_x = sel_x + sel_w * 0.68;
        let cam_y = sel_y + sel_h * 0.72;

        fframes::svgr!(
            <g opacity={enter_op}>
                // Step header
                <text x="160" y="102" font-family={FONT_MONO} font-weight="400" font-size="28" fill={ACCENT_RED}>
                    "STEP 01 · TARGET SELECTION"
                </text>
                <text x="160" y="158" font-family={FONT_DISPLAY} font-weight="700" font-size="52" fill={TEXT_WHITE}>
                    {step_title}
                </text>

                // Top Dynamic Island Pill
                <rect x="1360" y="68" width="400" height="54" rx="27" fill={PILL_BG} stroke="#343844" stroke-width="1.5" />
                <rect x="1370" y="76" width="102" height="38" rx="19" fill="#ffffff" fill-opacity={if in_window_mode { "0.0" } else { "0.18" }} />
                <text x="1421" y="101" text-anchor="middle" font-family={FONT_SANS} font-weight="500" font-size="20" fill={TEXT_WHITE}>"Area"</text>
                <text x="1555" y="101" text-anchor="middle" font-family={FONT_SANS} font-weight="500" font-size="20" fill={TEXT_SOFT}>"Full Screen"</text>
                <rect x="1648" y="83" width="1.5" height="24" fill="#2e323d" />
                <text x="1704" y="102" text-anchor="middle" font-family={FONT_MONO} font-weight="400" font-size="20" fill={TEXT_MUTED}>"x"</text>

                // Simulated Desktop Stage
                <rect x="160" y="200" width="1600" height="630" rx="22" fill={SURFACE} stroke={LINE_SUBTLE} stroke-width="2" />

                // Background Window A (Sidebar)
                <rect x="210" y="230" width="560" height="540" rx="14" fill={CARD_BG} stroke="#2a2f3a" stroke-width="1.5" />
                <text x="246" y="282" font-family={FONT_DISPLAY} font-weight="700" font-size="28" fill={TEXT_SOFT}>"Sessions"</text>
                <rect x="236" y="310" width="508" height="54" rx="10" fill="#232730" />
                <text x="260" y="345" font-family={FONT_MONO} font-weight="400" font-size="24" fill={TEXT_WHITE}>"DESKTOP · opencode"</text>
                <text x="260" y="415" font-family={FONT_MONO} font-weight="400" font-size="24" fill={TEXT_MUTED}>"APPARENT · rust + gpui"</text>
                <text x="260" y="475" font-family={FONT_MONO} font-weight="400" font-size="24" fill={TEXT_MUTED}>" IOS BUILD SCALE"</text>

                // Background Window B (Main Editor Window)
                <rect x="820" y="230" width="780" height="540" rx="16" fill={CARD_BG} stroke="#2a2f3a" stroke-width="1.5" />
                <text x="860" y="282" font-family={FONT_DISPLAY} font-weight="700" font-size="28" fill={TEXT_WHITE}>"OpenCode — Composer"</text>
                <rect x="860" y="315" width="700" height="160" rx="12" fill="#121419" />
                <text x="890" y="368" font-family={FONT_MONO} font-weight="400" font-size="24" fill={TEXT_SOFT}>"fn start_recording_with_target(&mut self)"</text>
                <text x="890" y="412" font-family={FONT_MONO} font-weight="400" font-size="24" fill={ACCENT_GREEN}>"// 4.2ms non-blocking startup"</text>

                // Dim everything outside the selection cutout inside the stage (160..1760, 200..830)
                <rect x="160" y="200" width="1600" height={(sel_y - 200.0).max(0.5)} fill="#000000" fill-opacity="0.52" />
                <rect x="160" y={sel_y + sel_h} width="1600" height={(830.0 - (sel_y + sel_h)).max(0.5)} fill="#000000" fill-opacity="0.52" />
                <rect x="160" y={sel_y} width={(sel_x - 160.0).max(0.5)} height={sel_h} fill="#000000" fill-opacity="0.52" />
                <rect x={sel_x + sel_w} y={sel_y} width={(1760.0 - (sel_x + sel_w)).max(0.5)} height={sel_h} fill="#000000" fill-opacity="0.52" />

                // Active Picker Highlight Box (Area -> Window)
                <rect x={sel_x} y={sel_y} width={sel_w} height={sel_h} rx={sel_rx}
                    fill={ACCENT_BLUE} fill-opacity={sel_fill_op}
                    stroke={sel_stroke} stroke-width="3" />

                // Dimension / Window Title Pill Badge
                <rect x={sel_x + sel_w * 0.5 - 140.0} y={sel_y + sel_h * 0.5 - 22.0} width="280" height="44" rx="22" fill="#18191df2" stroke="#ffffff33" stroke-width="1.5" />
                <text x={sel_x + sel_w * 0.5} y={sel_y + sel_h * 0.5 + 8.0} text-anchor="middle" font-family={FONT_MONO} font-weight="400" font-size="22" fill={TEXT_WHITE}>
                    {if in_window_mode {
                        "OpenCode - Composer".to_owned()
                    } else {
                        format!("{} x {}", sel_w.round() as i32, sel_h.round() as i32)
                    }}
                </text>

                // Floating Camera Cursor in Window Mode
                <g opacity={if in_window_mode { 1.0 } else { 0.0 }}>
                    <circle cx={cam_x} cy={cam_y} r="34" fill="#16171bf0" stroke="#ffffff55" stroke-width="2" />
                    <rect x={cam_x - 8.0} y={cam_y - 16.0} width="16" height="5" rx="2" fill={TEXT_WHITE} />
                    <rect x={cam_x - 18.0} y={cam_y - 11.0} width="36" height="24" rx="6" fill={TEXT_WHITE} />
                    <circle cx={cam_x} cy={cam_y + 1.0} r="7.5" fill="#16171b" />
                    <circle cx={cam_x} cy={cam_y + 1.0} r="3.5" fill={TEXT_WHITE} />
                </g>

                // Spacebar Keycap at Bottom
                {render_keycap(780.0, 880.0, 360.0, "Space  (Camera Mode)", space_press)}
            </g>
        )
    }
}

/// Scene 3 (9.4..14.8s): Click Ripple + Hold Shift for Thin Red tldraw Stroke + Voice Waveform
#[derive(Debug)]
pub struct DrawScene;

impl Scene for DrawScene {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(5.4)
    }

    fn overlap(&self) -> Overlap {
        Overlap::Previous(0.3)
    }

    fn render_frame<'a>(&'a self, frame: Frame, _ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.seconds();
        let enter_op = smoothstep(0.0, 0.25, t) * (1.0 - smoothstep(5.05, 5.38, t));

        // Click ripple at t = 0.8s
        let click_age = (t - 0.8).max(0.0);
        let ripple_op = if t >= 0.8 && t <= 1.8 {
            (1.0 - click_age).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let ripple_r = (12.0 + 36.0 * smoothstep(0.0, 0.8, click_age)).max(1.0);

        // Shift held during 1.6s .. 3.8s -> draws red tldraw curve, then fades out 4.2s .. 4.9s
        let shift_held = (smoothstep(1.5, 1.65, t) * (1.0 - smoothstep(3.8, 4.0, t))).clamp(0.0, 1.0);
        let draw_progress = smootherstep(1.65, 3.3, t);
        let stroke_fade = 1.0 - smoothstep(4.2, 4.85, t);
        let dash_len = 920.0_f32;
        let dash_offset = dash_len * (1.0 - draw_progress);

        // Animated voice waveform bars
        let bars: Vec<Svgr<'static>> = (0..28)
            .map(|i| {
                let phase = t * 8.5 + i as f32 * 0.55;
                let amp = (phase.sin().abs() * 0.65 + (phase * 0.47).cos().abs() * 0.35).clamp(0.15, 1.0);
                let bh = (8.0 + 34.0 * amp).max(4.0);
                let bx = 340.0 + i as f32 * 16.0;
                let by = 920.0 - bh * 0.5;
                fframes::svgr!(
                    <rect x={bx} y={by} width="8" height={bh} rx="4" fill={ACCENT_RED} />
                )
            })
            .collect();

        let rec_pulse = 0.65 + 0.35 * (t * 4.5).sin();

        fframes::svgr!(
            <g opacity={enter_op}>
                <text x="160" y="102" font-family={FONT_MONO} font-weight="400" font-size="28" fill={ACCENT_RED}>
                    "STEP 02 · SPEAK, CLICK & HOLD SHIFT TO DRAW"
                </text>
                <text x="160" y="158" font-family={FONT_DISPLAY} font-weight="700" font-size="52" fill={TEXT_WHITE}>
                    "2 · Hold Shift and Move to Draw Red Strokes"
                </text>

                // Recording Dynamic Island Pill
                <rect x="1380" y="68" width="380" height="54" rx="27" fill={PILL_BG} stroke="#343844" stroke-width="1.5" />
                <circle cx="1414" cy="95" r="14" fill={ACCENT_RED} fill-opacity={0.28 * rec_pulse} />
                <circle cx="1414" cy="95" r="6.5" fill={ACCENT_RED} />
                <text x="1465" y="102" text-anchor="middle" font-family={FONT_MONO} font-weight="400" font-size="22" fill={TEXT_WHITE}>
                    {format!("0:0{}", (t.floor() as u32).min(9))}
                </text>
                <rect x="1512" y="83" width="1.5" height="24" fill="#2e323d" />
                <text x="1566" y="101" text-anchor="middle" font-family={FONT_SANS} font-weight="500" font-size="20" fill={TEXT_SOFT}>"Cancel"</text>
                <rect x="1632" y="76" width="116" height="38" rx="19" fill={ACCENT_BLUE} />
                <text x="1690" y="101" text-anchor="middle" font-family={FONT_DISPLAY} font-weight="700" font-size="20" fill={TEXT_WHITE}>"Finish"</text>

                // Captured Window Canvas
                <rect x="160" y="200" width="1600" height="630" rx="22" fill={SURFACE} stroke="#ffffff38" stroke-width="2" />

                // Session rows being inspected
                <rect x="240" y="265" width="680" height="78" rx="14" fill="#232732" />
                <text x="280" y="314" font-family={FONT_DISPLAY} font-weight="700" font-size="30" fill={TEXT_WHITE}>"DESKTOP"</text>

                <rect x="240" y="385" width="680" height="78" rx="14" fill="#1c2029" />
                <text x="280" y="434" font-family={FONT_DISPLAY} font-weight="700" font-size="30" fill={TEXT_SOFT}>"SLACKBOT"</text>

                <rect x="240" y="525" width="680" height="78" rx="14" fill="#1c2029" />
                <text x="280" y="574" font-family={FONT_DISPLAY} font-weight="700" font-size="30" fill={TEXT_SOFT}>"BLOG GAME"</text>

                <rect x="240" y="645" width="680" height="78" rx="14" fill="#1c2029" />
                <text x="280" y="694" font-family={FONT_DISPLAY} font-weight="700" font-size="30" fill={TEXT_SOFT}>"APPARENT"</text>

                // Explanation Card on Right Side of Canvas
                <rect x="1020" y="265" width="660" height="458" rx="18" fill={CARD_BG} stroke={LINE_SUBTLE} stroke-width="1.5" />
                <text x="1064" y="332" font-family={FONT_MONO} font-weight="400" font-size="24" fill={ACCENT_RED}>"LIVE TELEMETRY"</text>
                <text x="1064" y="392" font-family={FONT_DISPLAY} font-weight="700" font-size="34" fill={TEXT_WHITE}>"• Clicks coalesce on final burst"</text>
                <text x="1064" y="452" font-family={FONT_DISPLAY} font-weight="700" font-size="34" fill={TEXT_WHITE}>"• Shift + move draws 4.5px red ink"</text>
                <text x="1064" y="512" font-family={FONT_DISPLAY} font-weight="700" font-size="34" fill={TEXT_WHITE}>"• Fades 650ms after releasing Shift"</text>
                <text x="1064" y="572" font-family={FONT_DISPLAY} font-weight="700" font-size="34" fill={TEXT_WHITE}>"• Words align to action [t0..t1]"</text>

                // Blue Click Ripple at (560, 480)
                <g opacity={ripple_op}>
                    <circle cx="560" cy="492" r={ripple_r} fill="#0088ff" fill-opacity="0.28" stroke="#ffffff" stroke-width="3" />
                    <circle cx="560" cy="492" r={(ripple_r + 5.0).max(1.0)} fill="none" stroke="#0088ff" stroke-width="2.5" />
                </g>

                // Smooth tldraw Red Marker Loop Drawn Around the Dead Space Between SLACKBOT & BLOG GAME
                <g opacity={stroke_fade}>
                    <path d="M 380 488 C 460 456, 740 456, 790 492 C 835 524, 730 548, 560 544 C 410 540, 345 512, 410 482"
                        fill="none"
                        stroke={ACCENT_RED}
                        stroke-width="6"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        stroke-dasharray={dash_len}
                        stroke-dashoffset={dash_offset} />
                </g>

                // Bottom Voice Bar + Shift Keycap
                <rect x="160" y="882" width="1290" height="76" rx="18" fill={CARD_BG} stroke={LINE_SUBTLE} stroke-width="1.5" />
                <text x="196" y="929" font-family={FONT_MONO} font-weight="400" font-size="24" fill={ACCENT_RED}>"MIC"</text>
                {bars}
                <text x="805" y="928" font-family={FONT_SANS} font-weight="500" font-size="25" fill={TEXT_WHITE}>
                    "“There's a dead space between these session tabs…”"
                </text>
                {render_keycap(1480.0, 888.0, 280.0, "⇧ Hold to Draw", shift_held)}
            </g>
        )
    }
}

/// Scene 4 (14.8..19.8s): Real Contact Sheet + Single-Pass Aligned Transcript
pub struct BundleScene<'a> {
    media: &'a ApparentDemoMedia,
}

impl std::fmt::Debug for BundleScene<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BundleScene").finish()
    }
}

impl Scene for BundleScene<'_> {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(5.0)
    }

    fn overlap(&self) -> Overlap {
        Overlap::Previous(0.3)
    }

    fn render_frame<'a>(&'a self, frame: Frame, _ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.seconds();
        let enter_op = smoothstep(0.0, 0.25, t) * (1.0 - smoothstep(4.65, 4.98, t));
        let card1_op = smoothstep(0.35, 0.75, t);
        let card2_op = smoothstep(0.65, 1.05, t);

        fframes::svgr!(
            <g opacity={enter_op}>
                <text x="160" y="102" font-family={FONT_MONO} font-weight="400" font-size="28" fill={ACCENT_GREEN}>
                    "STEP 03 · COMPACT AGENT BUNDLE"
                </text>
                <text x="160" y="158" font-family={FONT_DISPLAY} font-weight="700" font-size="52" fill={TEXT_WHITE}>
                    "3 · Labeled Frames in Image, Transcript in Text"
                </text>

                // Left: Real Contact Sheet Image from Apparent
                <g transform={frame.animate(fframes::timeline!(
                    at 0.1, animate Transform::translate(-40, 0) => Transform::translate(0, 0),
                        Easing::Spring { mass: 1.0, stiffness: 180.0, damping: 20.0 },
                ))}>
                    <rect x="160" y="210" width="760" height="740" rx="20" fill={CARD_BG} stroke={LINE_SUBTLE} stroke-width="2" />
                    <text x="200" y="262" font-family={FONT_MONO} font-weight="400" font-size="24" fill={TEXT_MUTED}>
                        "contact-sheet.png  (pure labeled grid)"
                    </text>
                    <image href={self.media.contact_sheet_png.href()} x="190" y="285" width="700" height="635" />
                </g>

                // Right: Single-pass Markdown Summary
                <g transform={frame.animate(fframes::timeline!(
                    at 0.22, animate Transform::translate(40, 0) => Transform::translate(0, 0),
                        Easing::Spring { mass: 1.0, stiffness: 180.0, damping: 20.0 },
                ))}>
                    <rect x="960" y="210" width="800" height="740" rx="20" fill={CARD_BG} stroke={LINE_SUBTLE} stroke-width="2" />
                    <text x="1000" y="262" font-family={FONT_MONO} font-weight="400" font-size="24" fill={TEXT_MUTED}>
                        "summary.md  (time-aligned transcript + paths)"
                    </text>
                    <text x="1000" y="326" font-family={FONT_MONO} font-weight="400" font-size="26" fill={ACCENT_GREEN}>
                        "[apparent 0:32 · OpenCode]"
                    </text>

                    // Keyframe #1 Block
                    <g opacity={card1_op}>
                        <rect x="996" y="365" width="728" height="220" rx="16" fill="#13151b" stroke="#2c313d" stroke-width="1.5" />
                        <rect x="1020" y="390" width="148" height="36" rx="18" fill="#261719" stroke={ACCENT_RED} stroke-width="1.5" />
                        <text x="1094" y="415" text-anchor="middle" font-family={FONT_MONO} font-weight="400" font-size="21" fill={TEXT_WHITE}>
                            "#1 · 0:15"
                        </text>
                        <text x="1188" y="415" font-family={FONT_MONO} font-weight="400" font-size="22" fill={TEXT_SOFT}>
                            "clicked (x 0.44, y 0.40)"
                        </text>
                        <text x="1020" y="468" font-family={FONT_SANS} font-weight="500" font-size="26" fill={TEXT_WHITE}>
                            "“As I drag between these session tabs,"
                        </text>
                        <text x="1020" y="506" font-family={FONT_SANS} font-weight="500" font-size="26" fill={TEXT_WHITE}>
                            "there's a little dead space between each one.”"
                        </text>
                        <text x="1020" y="555" font-family={FONT_MONO} font-weight="400" font-size="20" fill={TEXT_MUTED}>
                            "frames/frame-01.png"
                        </text>
                    </g>

                    // Keyframe #2 Block
                    <g opacity={card2_op}>
                        <rect x="996" y="615" width="728" height="220" rx="16" fill="#13151b" stroke="#2c313d" stroke-width="1.5" />
                        <rect x="1020" y="640" width="148" height="36" rx="18" fill="#261719" stroke={ACCENT_RED} stroke-width="1.5" />
                        <text x="1094" y="665" text-anchor="middle" font-family={FONT_MONO} font-weight="400" font-size="21" fill={TEXT_WHITE}>
                            "#2 · 0:17"
                        </text>
                        <text x="1188" y="665" font-family={FONT_MONO} font-weight="400" font-size="22" fill={TEXT_SOFT}>
                            "marked region x 0.41–0.44"
                        </text>
                        <text x="1020" y="718" font-family={FONT_SANS} font-weight="500" font-size="26" fill={TEXT_WHITE}>
                            "“It would be nice if it always highlighted"
                        </text>
                        <text x="1020" y="756" font-family={FONT_SANS} font-weight="500" font-size="26" fill={TEXT_WHITE}>
                            "the closest one in the sidebar.”"
                        </text>
                        <text x="1020" y="805" font-family={FONT_MONO} font-weight="400" font-size="20" fill={TEXT_MUTED}>
                            "frames/frame-02.png"
                        </text>
                    </g>

                    <text x="1000" y="902" font-family={FONT_MONO} font-weight="400" font-size="22" fill={ACCENT_BLUE}>
                        "Look closer: apparent frames 'latest' --from 0:00 --to 0:32"
                    </text>
                </g>
            </g>
        )
    }
}

/// Scene 5 (19.8..23.8s): ⌃⌥⇧V Two-Step Paste (Image + Transcript) into Agent Composer
#[derive(Debug)]
pub struct PasteScene;

impl Scene for PasteScene {
    fn duration(&self) -> Duration<'_> {
        Duration::Seconds(4.2)
    }

    fn overlap(&self) -> Overlap {
        Overlap::Previous(0.3)
    }

    fn render_frame<'a>(&'a self, frame: Frame, _ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.seconds();
        let enter_op = smoothstep(0.0, 0.25, t);
        let press = (smoothstep(0.65, 0.85, t) * (1.0 - smoothstep(1.15, 1.35, t))).clamp(0.0, 1.0);
        let step1_img_op = smoothstep(0.95, 1.25, t);
        let step2_txt_op = smoothstep(1.25, 1.60, t);

        let keys: Vec<Svgr<'static>> = [
            (690.0, 88.0, "⌃"),
            (792.0, 88.0, "⌥"),
            (894.0, 88.0, "⇧"),
            (996.0, 134.0, "V"),
        ]
        .into_iter()
        .map(|(kx, kw, label)| render_keycap(kx, 865.0, kw, label, press))
        .collect();

        fframes::svgr!(
            <g opacity={enter_op}>
                <text x="160" y="102" font-family={FONT_MONO} font-weight="400" font-size="28" fill={ACCENT_GREEN}>
                    "STEP 04 · TWO-STEP PASTE HANDOFF"
                </text>
                <text x="160" y="158" font-family={FONT_DISPLAY} font-weight="700" font-size="52" fill={TEXT_WHITE}>
                    "4 · Press Ctrl+Opt+Shift+V to Paste Both"
                </text>

                // Completed Dynamic Island Pill
                <rect x="1280" y="68" width="480" height="54" rx="27" fill={PILL_BG} stroke="#5cd69666" stroke-width="1.5" />
                <circle cx="1314" cy="95" r="14" fill="#34c75933" />
                <text x="1314" y="102" text-anchor="middle" font-family={FONT_MONO} font-weight="400" font-size="20" fill={ACCENT_GREEN}>"✓"</text>
                <text x="1380" y="101" text-anchor="middle" font-family={FONT_DISPLAY} font-weight="700" font-size="20" fill={TEXT_WHITE}>"Copied"</text>
                <text x="1508" y="101" text-anchor="middle" font-family={FONT_MONO} font-weight="400" font-size="19" fill={TEXT_MUTED}>"· 2 frames · ⌃⌥⇧V"</text>
                <rect x="1618" y="76" width="96" height="38" rx="19" fill={ACCENT_BLUE} />
                <text x="1666" y="101" text-anchor="middle" font-family={FONT_DISPLAY} font-weight="700" font-size="19" fill={TEXT_WHITE}>"Inspect"</text>
                <text x="1736" y="102" text-anchor="middle" font-family={FONT_MONO} font-weight="400" font-size="19" fill={TEXT_MUTED}>"x"</text>

                // Agent Composer Box
                <rect x="240" y="235" width="1440" height="560" rx="24" fill={CARD_BG} stroke="#343a48" stroke-width="2" />
                <text x="290" y="295" font-family={FONT_MONO} font-weight="400" font-size="24" fill={TEXT_MUTED}>
                    "OpenCode Prompt Composer"
                </text>

                // Step 1: Attached Image Chip
                <g opacity={step1_img_op}>
                    <rect x="290" y="330" width="495" height="72" rx="16" fill="#1f2637" stroke={ACCENT_BLUE} stroke-width="2" />
                    <text x="322" y="375" font-family={FONT_MONO} font-weight="400" font-size="25" fill={TEXT_WHITE}>
                        "[Image 1: contact-sheet.png]"
                    </text>
                    <text x="815" y="375" font-family={FONT_MONO} font-weight="400" font-size="22" fill={ACCENT_GREEN}>
                        "← 1st ⌘V attaches contact sheet"
                    </text>
                </g>

                // Step 2: Pasted Transcript & Keyframe Paths
                <g opacity={step2_txt_op}>
                    <rect x="290" y="428" width="1340" height="320" rx="16" fill="#12141a" stroke="#282d38" stroke-width="1.5" />
                    <text x="325" y="478" font-family={FONT_MONO} font-weight="400" font-size="24" fill={ACCENT_GREEN}>
                        "[apparent 0:32 · OpenCode]   ← 2nd ⌘V (160ms later) inserts aligned text"
                    </text>
                    <text x="325" y="532" font-family={FONT_MONO} font-weight="400" font-size="23" fill={TEXT_WHITE}>
                        "- [#1 · 0:15] clicked (x 0.44, y 0.40) — “As I drag between these session tabs…”"
                    </text>
                    <text x="325" y="582" font-family={FONT_MONO} font-weight="400" font-size="23" fill={TEXT_WHITE}>
                        "- [#2 · 0:17] marked region x 0.41–0.44 — “It would be nice if it always highlighted…”"
                    </text>
                    <text x="325" y="642" font-family={FONT_MONO} font-weight="400" font-size="21" fill={TEXT_MUTED}>
                        "Video: ~/Library/Application Support/apparent/captures/…/recording.mp4"
                    </text>
                    <text x="325" y="688" font-family={FONT_MONO} font-weight="400" font-size="21" fill={ACCENT_BLUE}>
                        "Look closer: apparent frames 'latest' --from 0:00 --to 0:32 --fps 4"
                    </text>
                </g>

                // Bottom Shortcut Keycaps
                {keys}
            </g>
        )
    }
}
