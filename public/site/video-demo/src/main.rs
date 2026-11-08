use fframes::{EncoderOptions, RenderOptions, StaticMediaProvider, cli, cli::clap};
use fframes_skia_renderer::{
    SkiaFFramesRenderer, SkiaPipelineConcurrencyPolicy, SkiaPipelineConfig, metal::SkiaMetalCtx,
};
use apparent_demo::{HEIGHT, ApparentDemoMedia, ApparentDemoVideo, WIDTH};
use std::process::ExitCode;

#[derive(Debug, clap::Args)]
struct VideoArgs {
    #[arg(long, default_value = "Apparent", global = true)]
    title: String,
}

fn main() -> ExitCode {
    let args = cli::parse::<VideoArgs>();
    let media = ApparentDemoMedia::prepare().expect("media");
    let title = args.app.title.clone();
    let video = ApparentDemoVideo::new(&media, &title);
    let gpu = SkiaMetalCtx::new(WIDTH, HEIGHT).expect("GPU context");

    cli::new(
        &video,
        RenderOptions {
            media: Some(&media),
            video_encoder_options: EncoderOptions {
                preferred_encoder: Some("libx264"),
                codec_params: Some(&[("crf", "18"), ("preset", "medium"), ("tune", "animation")]),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .args(args)
    .backend(
        SkiaFFramesRenderer::new_metal(
            &gpu,
            SkiaPipelineConfig {
                concurrency_policy: SkiaPipelineConcurrencyPolicy::MaxPerformance,
                ..Default::default()
            },
        )
        .expect("skia renderer"),
    )
    .preview(fframes_native_player::cli_preview)
    .run()
}
