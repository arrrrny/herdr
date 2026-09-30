use super::*;
use base64::Engine as _;

fn fixture() -> Vec<u8> {
    fixture_size(32, 16)
}

fn fixture_size(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![42; (width * height * 4) as usize])
            .unwrap();
    }
    bytes
}

fn terminal() -> Terminal {
    let mut terminal = Terminal::new(20, 10, 0).unwrap();
    terminal.enable_kitty_graphics().unwrap();
    terminal.set_kitty_png_forwarding(true).unwrap();
    terminal.resize(20, 10, 8, 16).unwrap();
    terminal
}

#[test]
#[ignore = "manual fixed-geometry native PNG ingestion/extraction scaling profile"]
fn native_png_render_scale_profile() {
    let png = fixture_size(800, 480);
    let upload = format!(
        "\x1b[H\x1b_Ga=T,f=100,i=7,c=60,r=20,C=1,q=2;{}\x1b\\",
        base64::engine::general_purpose::STANDARD.encode(&png)
    );
    for count in [1, 15] {
        for forwarding in [false, true] {
            let mut panes = (0..count)
                .map(|_| {
                    let mut pane = Terminal::new(120, 40, 0).unwrap();
                    pane.enable_kitty_graphics().unwrap();
                    pane.set_kitty_png_forwarding(forwarding).unwrap();
                    pane.resize(120, 40, 8, 16).unwrap();
                    pane
                })
                .collect::<Vec<_>>();
            let mut samples = Vec::new();
            for iteration in 0..65 {
                let begin = std::time::Instant::now();
                for pane in &mut panes {
                    pane.write(upload.as_bytes());
                    let placements = pane.kitty_image_placements().unwrap();
                    assert_eq!(placements.len(), 1);
                    assert_eq!(
                        placements[0].data.len(),
                        if forwarding { png.len() } else { 800 * 480 * 4 }
                    );
                    std::hint::black_box(placements);
                }
                if iteration >= 5 {
                    samples.push(begin.elapsed().as_micros());
                }
            }
            samples.sort_unstable();
            eprintln!("native PNG panes={count} forwarding={forwarding} median_us={} p95_us={} png_bytes={}", samples[30], samples[57], png.len());
        }
    }
}

#[test]
fn normal_kitty_graphics_fully_decodes_quiet_png_uploads() {
    let mut terminal = Terminal::new(20, 10, 0).unwrap();
    terminal.enable_kitty_graphics().unwrap();
    terminal.resize(20, 10, 8, 16).unwrap();
    let before = PNG_DECODE_CALLS.get();
    terminal.write(
        format!(
            "\x1b_Ga=T,f=100,i=7,c=4,r=1,q=2;{}\x1b\\",
            base64::engine::general_purpose::STANDARD.encode(fixture())
        )
        .as_bytes(),
    );
    let placements = terminal.kitty_image_placements().unwrap();
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].format, KittyImageFormat::Rgba);
    assert_eq!(placements[0].data, vec![42; 32 * 16 * 4]);
    assert_eq!(PNG_DECODE_CALLS.get(), before + 1);
}

#[test]
fn quiet_png_forwarding_retains_exact_payload_without_pixel_decode() {
    let bytes = fixture();
    let mut terminal = terminal();
    let before = PNG_DECODE_CALLS.get();
    terminal.write(
        format!(
            "\x1b_Ga=T,f=100,i=7,c=4,r=1,q=2;{}\x1b\\",
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        )
        .as_bytes(),
    );
    let placements = terminal.kitty_image_placements().unwrap();
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].format, KittyImageFormat::Png);
    assert_eq!(placements[0].data, bytes);
    assert_eq!(
        (placements[0].image_width, placements[0].image_height),
        (32, 16)
    );
    assert_eq!(PNG_DECODE_CALLS.get(), before);
}

#[test]
fn placeholder_png_forwarding_and_lazy_animation_decode() {
    let bytes = fixture();
    let mut terminal = terminal();
    let before = PNG_DECODE_CALLS.get();
    terminal.write(
        format!(
            "\x1b_Ga=T,f=100,U=1,i=7,c=4,r=1,q=2;{}\x1b\\",
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        )
        .as_bytes(),
    );
    terminal.write("\x1b[H\x1b[38;2;0;0;7m\u{10eeee}\u{0305}\u{0305}\x1b[0m".as_bytes());
    let placements = terminal.kitty_image_placements().unwrap();
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].format, KittyImageFormat::Png);
    assert_eq!(placements[0].data, bytes);
    assert_eq!(PNG_DECODE_CALLS.get(), before);
    let fingerprint = placements[0].data_fingerprint;
    terminal.write(b"\x1b_Ga=a,i=7,q=2;\x1b\\");
    let materialized = terminal.kitty_image_placements().unwrap();
    assert_eq!(PNG_DECODE_CALLS.get(), before + 1);
    assert_eq!(materialized[0].format, KittyImageFormat::Rgba);
    assert_eq!(materialized[0].data, [42; 32 * 16 * 4]);
    assert_ne!(materialized[0].data_fingerprint, fingerprint);
}

#[test]
fn experimental_quiet_png_defers_crc_valid_compressed_data_errors() {
    let mut bytes = fixture();
    let mut offset = 8;
    loop {
        let len = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        if &bytes[offset + 4..offset + 8] == b"IDAT" {
            bytes[offset + 8] = 0; // Invalid zlib header, but structurally valid PNG chunks.
            let mut crc = !0u32;
            for &byte in &bytes[offset + 4..offset + 8 + len] {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ (0xedb88320u32 & 0u32.wrapping_sub(crc & 1));
                }
            }
            bytes[offset + 8 + len..offset + 12 + len].copy_from_slice(&(!crc).to_be_bytes());
            break;
        }
        offset += len + 12;
    }
    for quiet in [0, 2] {
        let mut terminal = terminal();
        let before = PNG_DECODE_CALLS.get();
        terminal.write(
            format!(
                "\x1b_Ga=T,f=100,i=7,c=4,r=1,q={quiet};{}\x1b\\",
                base64::engine::general_purpose::STANDARD.encode(&bytes)
            )
            .as_bytes(),
        );
        let placements = terminal.kitty_image_placements().unwrap();
        if quiet == 0 {
            assert!(placements.is_empty());
            assert_eq!(PNG_DECODE_CALLS.get(), before + 1);
        } else {
            assert_eq!(placements[0].data, bytes);
            assert_eq!(PNG_DECODE_CALLS.get(), before);
            terminal.write(b"\x1b_Ga=a,i=7,q=2;\x1b\\");
            assert_eq!(PNG_DECODE_CALLS.get(), before + 1);
            assert_eq!(terminal.kitty_image_placements().unwrap()[0].data, bytes);
        }
    }
}

#[test]
fn response_bearing_png_and_queries_keep_full_validation() {
    let bytes = fixture();
    for (action, quiet) in [('T', 0), ('T', 1), ('q', 2)] {
        let mut terminal = terminal();
        let before = PNG_DECODE_CALLS.get();
        terminal.write(
            format!(
                "\x1b_Ga={action},f=100,i=7,c=4,r=1,q={quiet};{}\x1b\\",
                base64::engine::general_purpose::STANDARD.encode(&bytes)
            )
            .as_bytes(),
        );
        assert_eq!(
            PNG_DECODE_CALLS.get(),
            before + 1,
            "action {action}, quiet {quiet}"
        );
        let placements = terminal.kitty_image_placements().unwrap();
        if action == 'q' {
            assert!(placements.is_empty());
        } else {
            assert_eq!(placements[0].format, KittyImageFormat::Rgba);
        }
    }
}

#[test]
fn chunked_png_forwarding_inherits_quiet_mode() {
    let bytes = fixture();
    let mut terminal = terminal();
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let split = (encoded.len() / 8) * 4;
    let before = PNG_DECODE_CALLS.get();
    terminal.write(
        format!(
            "\x1b_Ga=T,f=100,i=7,c=4,r=1,q=2,m=1;{}\x1b\\",
            &encoded[..split]
        )
        .as_bytes(),
    );
    assert!(terminal.kitty_image_placements().unwrap().is_empty());
    terminal.write(format!("\x1b_Gm=0;{}\x1b\\", &encoded[split..]).as_bytes());
    assert_eq!(terminal.kitty_image_placements().unwrap()[0].data, bytes);
    assert_eq!(PNG_DECODE_CALLS.get(), before);
}

/// Encode a PNG whose IHDR declares `width` x `height` while carrying only a
/// single row of real pixel data.
///
/// This is the shape of the denial-of-service input: a few hundred bytes that
/// claim an enormous frame. The decoder sizes its output buffer from the header,
/// so an unvalidated width/height turns a tiny upload into a multi-gigabyte
/// allocation inside the server.
fn header_only_png(width: u32, height: u32) -> Vec<u8> {
    /// Standard CRC-32 (IEEE 802.3), spelled out here so this test does not need
    /// the decoder crate to also provide an encoder for its input side.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }

    fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut body = Vec::from(*kind);
        body.extend_from_slice(payload);
        let mut out = Vec::from((payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    }

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace

    // A real, minimal zlib stream for an empty payload. Without an IDAT the
    // decoder stops at `read_info()` with `MissingImageData` and never computes
    // `output_buffer_size()`, which would make this test reject the input for
    // the wrong reason and prove nothing about the bound.
    const EMPTY_ZLIB: &[u8] = &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01];

    let mut png = Vec::from(*b"\x89PNG\r\n\x1a\n");
    png.extend_from_slice(&chunk(b"IHDR", &ihdr));
    png.extend_from_slice(&chunk(b"IDAT", EMPTY_ZLIB));
    png.extend_from_slice(&chunk(b"IEND", &[]));
    png
}

#[test]
fn oversized_declared_dimensions_are_rejected_before_allocation() {
    // ~16 GiB of RGBA if the header were trusted. The point of the test is that
    // this returns promptly and cleanly rather than asking the allocator for it.
    assert!(decode_png_rgba(&header_only_png(65_535, 65_535)).is_none());
    assert!(decode_png_rgba(&header_only_png(40_000, 40_000)).is_none());
    assert!(decode_png_rgba(&header_only_png(1, 20_000_000)).is_none());
    assert!(decode_png_rgba(&header_only_png(20_000_000, 1)).is_none());
}

#[test]
fn zero_dimensions_are_rejected() {
    assert!(decode_png_rgba(&header_only_png(0, 16)).is_none());
    assert!(decode_png_rgba(&header_only_png(16, 0)).is_none());
    assert!(decode_png_rgba(&header_only_png(0, 0)).is_none());
}

#[test]
fn dimension_validation_accepts_the_house_maximum() {
    let max = 16 * 1024 * 1024;
    assert_eq!(validate_image_dimensions(4096, 4096), Some(4096 * 4096));
    assert_eq!(
        validate_image_dimensions(16_384, 1_024),
        Some(16_384 * 1_024)
    );
    // Exactly at the pixel ceiling, and one past it.
    assert_eq!(validate_image_dimensions(4096, 4096), Some(max));
    assert_eq!(validate_image_dimensions(8192, 2048), Some(max));
    assert_eq!(validate_image_dimensions(4096, 4097), None);
    // The per-side cap bites even when the product would fit.
    assert_eq!(validate_image_dimensions(16_385, 1), None);
    assert_eq!(validate_image_dimensions(1, 16_385), None);
}

#[test]
fn decoder_survives_hostile_png_bytes() {
    // Random bytes must not panic; the trampoline's catch_unwind depends on the
    // decoder treating malformed input as "no image".
    let noise: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    assert!(decode_png_rgba(&noise).is_none());
    // A truncated but signature-valid PNG.
    let mut truncated = header_only_png(64, 64);
    truncated.truncate(20);
    assert!(decode_png_rgba(&truncated).is_none());
}
#[test]
fn hostile_header_really_reaches_the_allocation_point() {
    // Guards the test above against becoming vacuous. If `read_info()` ever
    // stopped accepting this input, the bounds test would still pass while
    // proving nothing — the decoder would simply be rejecting the file for an
    // unrelated reason. Pin that the header parses and that the buffer the old
    // code allocated from it really is ~16 GiB.
    let bytes = header_only_png(65_535, 65_535);
    let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let reader = decoder
        .read_info()
        .expect("read_info must succeed for the bound to be what rejects this input");
    assert_eq!(
        (reader.info().width, reader.info().height),
        (65_535, 65_535)
    );
    let size = reader.output_buffer_size();
    assert!(
        size > 1024 * 1024 * 1024,
        "expected a multi-gigabyte pre-fix allocation, got {size}"
    );
    // And the guarded decoder refuses it anyway, without attempting that size.
    assert!(decode_png_rgba(&bytes).is_none());
}
