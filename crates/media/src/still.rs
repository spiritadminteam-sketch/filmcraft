//! Still images (PNG, JPEG, GIF, WebP, TIFF, BMP, Targa, DPX) as media sources.
//!
//! An animated GIF stays a still clip (it can be made any length on the timeline, like any
//! picture) but plays its animation, looping, at the frame delays stored in the file.

use std::sync::Arc;

use filmcraft_frame::VideoFrame;
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick};

use crate::{FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, Result, VideoStreamInfo};

/// Default still duration (Premiere's preference default is 5 seconds).
pub const DEFAULT_STILL_DURATION: Tick = Tick(5 * TICKS_PER_SECOND);

/// Whether `b` (the contents of the file `name`) is a still image we decode.
pub fn sniff(name: &str, b: &[u8]) -> bool {
    crate::dpx::sniff(b)
        || crate::tga::sniff(name, b)
        || b.starts_with(b"\x89PNG")
        || b.starts_with(&[0xFF, 0xD8, 0xFF])
        || b.starts_with(b"GIF8")
        || (b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP")
        || b.starts_with(b"II*\0")
        || b.starts_with(b"MM\0*")
        || b.starts_with(b"BM")
}

/// A decoded picture: straight-alpha RGBA8, top row first.
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Whether any pixel is not fully opaque.
    pub has_alpha: bool,
}

/// Decode a still image file (`name` picks Targa, which has no magic number; the content picks
/// every other format).
pub fn decode_raster(name: &str, bytes: &[u8]) -> Result<Raster> {
    if crate::dpx::sniff(bytes) {
        return crate::dpx::decode(name, bytes);
    }
    if crate::tga::sniff(name, bytes) {
        return crate::tga::decode(name, bytes);
    }
    let img = image::load_from_memory(bytes).map_err(|e| MediaError::Decode(format!("{name}: {e}")))?;
    let has_alpha = img.color().has_alpha();
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(Raster { width, height, rgba: rgba.into_raw(), has_alpha })
}

pub struct StillSource {
    info: MediaInfo,
    frame: Arc<VideoFrame>,
    /// An animated GIF's frames after the first: (start time within one loop, picture).
    /// Empty for every other still.
    animation: Vec<(Tick, Arc<VideoFrame>)>,
    /// One loop of the animation (zero when not animated).
    loop_len: Tick,
}

/// At most this many animation frames are kept...
const MAX_GIF_FRAMES: usize = 2000;
/// ...and at most this many bytes of decoded pictures; the rest of a longer GIF is dropped
/// (it then loops over what was kept).
const MAX_GIF_BYTES: usize = 1 << 30;

/// The display time of a GIF frame with delay `ms`: like browsers, a delay under 20 ms
/// (0 and 10 ms are common in the wild) shows for 100 ms.
fn gif_delay(ms: u32) -> Tick {
    let ms = if ms < 20 { 100 } else { ms.min(60_000) };
    Tick(i64::from(ms) * (TICKS_PER_SECOND / 1000))
}

/// The frames of an animated GIF with their start times, and the loop length. `None` for a
/// GIF with a single frame or one that does not decode as an animation.
#[allow(clippy::type_complexity)]
fn gif_animation(bytes: &[u8]) -> Option<(Vec<(Tick, u32, u32, Vec<u8>)>, Tick)> {
    use image::AnimationDecoder;
    if !bytes.starts_with(b"GIF8") {
        return None;
    }
    let dec = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let mut out = Vec::new();
    let mut at = Tick(0);
    let mut total = 0usize;
    for f in dec.into_frames() {
        let Ok(f) = f else { break };
        let (num, den) = f.delay().numer_denom_ms();
        let ms = num.checked_div(den).unwrap_or(0);
        let buf = f.into_buffer();
        let (w, h) = buf.dimensions();
        total = total.saturating_add(buf.as_raw().len());
        if out.len() >= MAX_GIF_FRAMES || total > MAX_GIF_BYTES {
            break;
        }
        out.push((at, w, h, buf.into_raw()));
        at = Tick(at.0.saturating_add(gif_delay(ms).0));
    }
    if out.len() < 2 || at.0 <= 0 {
        return None;
    }
    Some((out, at))
}

impl StillSource {
    pub fn decode(name: &str, bytes: &[u8]) -> Result<Self> {
        let r = decode_raster(name, bytes)?;
        let mut src = Self::from_rgba(name, r.width, r.height, r.rgba, r.has_alpha, bytes.len() as u64);
        if let Some((frames, loop_len)) = gif_animation(bytes) {
            // every frame is the full canvas, the size of the first picture
            let frames: Vec<_> = frames
                .into_iter()
                .filter(|(_, w, h, _)| (*w, *h) == (r.width, r.height))
                .map(|(at, w, h, px)| (at, Arc::new(VideoFrame::rgba8(w, h, px))))
                .collect();
            if frames.len() >= 2 {
                src.animation = frames;
                src.loop_len = loop_len;
                if let Some(v) = src.info.video.as_mut() {
                    v.has_alpha = true;
                    v.pixel_format = "RGBA 8-bit".into();
                }
            }
        }
        Ok(src)
    }

    /// Whether this is an animated GIF.
    pub fn is_animated(&self) -> bool {
        !self.animation.is_empty()
    }

    pub fn from_rgba(name: &str, w: u32, h: u32, rgba: Vec<u8>, has_alpha: bool, file_size: u64) -> Self {
        let codec = name.rsplit('.').next().unwrap_or("image").to_ascii_uppercase();
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Still,
            duration: DEFAULT_STILL_DURATION,
            video: Some(VideoStreamInfo {
                width: w,
                height: h,
                frame_rate: FrameRate::FPS_29_97,
                par: (1, 1),
                codec,
                pixel_format: if has_alpha { "RGBA 8-bit".into() } else { "RGB 8-bit".into() },
                color: filmcraft_color::ColorInfo::SRGB_FULL,
                has_alpha,
                bitrate: None,
                hdr: None,
            }),
            audio_streams: Vec::new(),
            container: "Image".into(),
            start_timecode: None,
            file_size: Some(file_size),
        };
        Self { info, frame: Arc::new(VideoFrame::rgba8(w, h, rgba)), animation: Vec::new(), loop_len: Tick(0) }
    }
}

impl MediaSource for StillSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, req: FrameRequest) -> Result<Arc<VideoFrame>> {
        if self.animation.is_empty() || self.loop_len.0 <= 0 {
            return Ok(self.frame.clone());
        }
        // the animation loops for as long as the clip lasts
        let t = req.time.0.rem_euclid(self.loop_len.0);
        let i = self.animation.partition_point(|(at, _)| at.0 <= t);
        let f = i.checked_sub(1).and_then(|i| self.animation.get(i)).map_or(&self.frame, |(_, f)| f);
        Ok(f.clone())
    }
    fn audio(&self, _start: i64, _frames: usize, _sample_rate: u32) -> Result<filmcraft_frame::AudioBuffer> {
        Err(MediaError::NoStream("audio"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GIF of `colors` (one 4×2 frame each) with `delay_ms` per frame.
    fn gif(colors: &[[u8; 4]], delay_ms: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
            let frames = colors.iter().map(|c| {
                let buf = image::RgbaImage::from_pixel(4, 2, image::Rgba(*c));
                image::Frame::from_parts(buf, 0, 0, image::Delay::from_numer_denom_ms(delay_ms, 1))
            });
            enc.encode_frames(frames).unwrap();
        }
        out
    }

    fn first_px(s: &StillSource, ms: i64) -> [u8; 4] {
        let f = s.video_frame(FrameRequest::full(Tick(ms * (TICKS_PER_SECOND / 1000)))).unwrap();
        match &f.data {
            filmcraft_frame::PixelData::Rgba8(p) => [p[0], p[1], p[2], p[3]],
            _ => panic!("not RGBA"),
        }
    }

    #[test]
    fn an_animated_gif_plays_and_loops() {
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        let green = [0, 255, 0, 255];
        let s = StillSource::decode("a.gif", &gif(&[red, blue, green], 100)).unwrap();
        assert!(s.is_animated());
        assert_eq!(s.info().kind, MediaKind::Still);
        assert_eq!(first_px(&s, 0), red);
        assert_eq!(first_px(&s, 50), red);
        assert_eq!(first_px(&s, 150), blue);
        assert_eq!(first_px(&s, 250), green);
        // loops: 300 ms is the start again
        assert_eq!(first_px(&s, 320), red);
        assert_eq!(first_px(&s, 1_050), blue);
        // a time before the start (a trimmed or reversed clip) still lands in the loop
        assert_eq!(first_px(&s, -50), green);
    }

    #[test]
    fn zero_delays_play_at_ten_frames_a_second_and_one_frame_gifs_stay_still() {
        let s = StillSource::decode("z.gif", &gif(&[[1, 2, 3, 255], [4, 5, 6, 255]], 0)).unwrap();
        assert!(s.is_animated());
        assert_eq!(first_px(&s, 90), [1, 2, 3, 255]);
        assert_eq!(first_px(&s, 110), [4, 5, 6, 255]);
        let one = StillSource::decode("o.gif", &gif(&[[9, 9, 9, 255]], 50)).unwrap();
        assert!(!one.is_animated());
        assert_eq!(first_px(&one, 500), [9, 9, 9, 255]);
    }

    #[test]
    fn garbage_after_the_gif_header_is_an_error_not_a_crash() {
        assert!(StillSource::decode("bad.gif", b"GIF89a\x01\x00").is_err());
        assert!(gif_animation(b"GIF89a").is_none());
    }
}
