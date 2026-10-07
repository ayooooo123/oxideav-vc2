//! The registry decoder reports the size and pixel layout of the frame it
//! last returned (oxideav-core `Decoder::output_video_dimensions` /
//! `output_pixel_format`): odd sizes and 10-bit included. A new sequence
//! is reported with its first frame, not when its header is parsed.
//!
//! §11.6.2 halves an odd subsampled luma size rounding down, where the
//! core layout rounds up: such a frame reports its size and no format.
//!
//! `decoded_format/*.vc2` are two-picture FFmpeg `vc2` encodes of
//! `testsrc`.

use oxideav_core::{
    CodecId, CodecParameters, Decoder, Error, Frame, Packet, PixelFormat, TimeBase, VideoFrame,
};

/// A stream and the frames it decodes to.
struct Stream {
    bytes: &'static [u8],
    size: (u32, u32),
    /// Chroma plane width × height.
    chroma: (u32, u32),
    bytes_per_sample: usize,
    /// `None` where the chroma planes match no core layout.
    format: Option<PixelFormat>,
}

const STREAMS: [Stream; 3] = [
    Stream {
        bytes: include_bytes!("decoded_format/a_444_33x17.vc2"),
        size: (33, 17),
        chroma: (33, 17),
        bytes_per_sample: 1,
        format: Some(PixelFormat::Yuv444P),
    },
    Stream {
        bytes: include_bytes!("decoded_format/b_420p10_34x18.vc2"),
        size: (34, 18),
        chroma: (17, 9),
        bytes_per_sample: 2,
        format: Some(PixelFormat::Yuv420P10Le),
    },
    // 4:2:2 at an odd width: 17-wide chroma, where `Yuv422P` has 18.
    Stream {
        bytes: include_bytes!("decoded_format/c_422_35x19.vc2"),
        size: (35, 19),
        chroma: (17, 19),
        bytes_per_sample: 1,
        format: None,
    },
];

const PICTURES_PER_STREAM: usize = 2;

fn decoder() -> Box<dyn Decoder> {
    oxideav_vc2::decoder::make_decoder(&CodecParameters::video(CodecId::new("vc2")))
        .expect("decoder")
}

fn report(dec: &dyn Decoder) -> (Option<(u32, u32)>, Option<PixelFormat>) {
    (dec.output_video_dimensions(), dec.output_pixel_format())
}

fn assert_frame(dec: &dyn Decoder, frame: &VideoFrame, want: &Stream, at: usize) {
    assert_eq!(
        report(dec),
        (Some(want.size), want.format),
        "report after frame {at}"
    );
    let planes = frame.image_planes();
    assert_eq!(planes.len(), 3, "frame {at}: planes");
    for (i, plane) in planes.iter().enumerate() {
        let (pw, ph) = if i == 0 { want.size } else { want.chroma };
        assert_eq!(
            plane.stride,
            pw as usize * want.bytes_per_sample,
            "frame {at}: plane {i} stride"
        );
        assert_eq!(
            plane.data.len(),
            plane.stride * ph as usize,
            "frame {at}: plane {i} rows"
        );
    }
}

/// Receives every available frame, checking each against the stream it
/// belongs to right after it is returned.
fn receive_all(dec: &mut dyn Decoder, seen: &mut usize) {
    loop {
        match dec.receive_frame() {
            Ok(Frame::Video(frame)) => {
                assert_frame(dec, &frame, &STREAMS[*seen / PICTURES_PER_STREAM], *seen);
                *seen += 1;
            }
            Ok(_) => panic!("non-video frame"),
            Err(Error::NeedMore | Error::Eof) => return,
            Err(e) => panic!("receive: {e}"),
        }
    }
}

/// All three sequences in one packet: every picture is decoded before
/// the first frame is returned.
#[test]
fn each_frame_reports_its_own_size_and_layout() {
    let stream: Vec<u8> = STREAMS
        .iter()
        .flat_map(|s| s.bytes.iter().copied())
        .collect();
    let mut dec = decoder();
    dec.send_packet(&Packet::new(0, TimeBase::new(1, 25), stream))
        .expect("send");
    assert_eq!(
        report(&*dec),
        (Some(STREAMS[0].size), STREAMS[0].format),
        "before the first frame"
    );
    let mut seen = 0;
    receive_all(&mut *dec, &mut seen);
    dec.flush().expect("flush");
    receive_all(&mut *dec, &mut seen);
    assert_eq!(seen, STREAMS.len() * PICTURES_PER_STREAM, "frames");
}

/// One sequence per packet: a new sequence changes nothing until its
/// first frame is returned.
#[test]
fn reports_change_with_the_frame_that_carries_the_change() {
    let mut dec = decoder();
    let mut seen = 0;
    for (k, s) in STREAMS.iter().enumerate() {
        dec.send_packet(&Packet::new(0, TimeBase::new(1, 25), s.bytes.to_vec()))
            .expect("send");
        if k > 0 {
            let before = &STREAMS[k - 1];
            assert_eq!(
                report(&*dec),
                (Some(before.size), before.format),
                "sequence {k}: before its frames"
            );
        }
        receive_all(&mut *dec, &mut seen);
    }
    assert_eq!(seen, STREAMS.len() * PICTURES_PER_STREAM, "frames");
}
