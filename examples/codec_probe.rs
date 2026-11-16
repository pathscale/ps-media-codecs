use std::{
    error::Error,
    ffi::OsStr,
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use ps_media_codecs::{
    OpusCodec, Vp9Decoder, Vp9Encoder, Yuv420Frame, MAX_OPUS_PACKET_BYTES, MAX_VIDEO_PACKET_BYTES,
    OPUS_FRAME_SAMPLES, OPUS_SAMPLE_RATE,
};

const OPUS_CHANNELS: usize = 2;
const OPUS_FRAMES: usize = 5;
const VIDEO_WIDTH: u32 = 64;
const VIDEO_HEIGHT: u32 = 48;
const VIDEO_FRAMES: usize = 4;
const OPUS_FRAME_DURATION_MICROS: i64 = 20_000;
const VIDEO_FRAME_DURATION_MICROS: i64 = 33_333;

struct Vp9Packet {
    timestamp_micros: i64,
    chunk_type: &'static str,
    data: Vec<u8>,
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn require(condition: bool, message: &'static str) -> Result<(), io::Error> {
    if condition {
        Ok(())
    } else {
        Err(invalid_data(message))
    }
}

fn parse_export_dir() -> Result<Option<PathBuf>, io::Error> {
    let mut args = std::env::args_os().skip(1);
    let Some(flag) = args.next() else {
        return Ok(None);
    };
    if flag.as_os_str() != OsStr::new("--export-dir") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: codec_probe [--export-dir ABSOLUTE_PATH]",
        ));
    }
    let path = args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "--export-dir requires an absolute path",
        )
    })?;
    if args.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: codec_probe [--export-dir ABSOLUTE_PATH]",
        ));
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--export-dir requires an absolute path",
        ));
    }
    Ok(Some(path))
}

fn refuse_vp9_environment_overrides() -> Result<(), io::Error> {
    if std::env::vars_os().any(|(name, _)| name.to_string_lossy().starts_with("VP9_")) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "VP9_* environment overrides are present; clear them for this default-profile probe",
        ));
    }
    Ok(())
}

fn run_opus_probe() -> Result<Vec<Vec<u8>>, Box<dyn Error>> {
    let mut codec = OpusCodec::new(OPUS_CHANNELS)?;
    let mut packet = [0_u8; MAX_OPUS_PACKET_BYTES];
    let mut decoded = vec![0.0_f32; OPUS_FRAME_SAMPLES * OPUS_CHANNELS];
    let mut total_packet_bytes = 0_usize;
    let mut packets = Vec::with_capacity(OPUS_FRAMES);

    for frame_index in 0..OPUS_FRAMES {
        let mut pcm = vec![0.0_f32; OPUS_FRAME_SAMPLES * OPUS_CHANNELS];
        for sample_index in 0..OPUS_FRAME_SAMPLES {
            let absolute_sample = frame_index * OPUS_FRAME_SAMPLES + sample_index;
            let time = absolute_sample as f32 / OPUS_SAMPLE_RATE as f32;
            pcm[sample_index * OPUS_CHANNELS] = (std::f32::consts::TAU * 440.0 * time).sin() * 0.2;
            pcm[sample_index * OPUS_CHANNELS + 1] =
                (std::f32::consts::TAU * 660.0 * time).sin() * 0.2;
        }

        let packet_len = codec.encode_20ms(&pcm, &mut packet)?;
        require(packet_len > 0, "Opus encoder returned an empty packet")?;
        let decoded_samples = codec.decode_20ms(&packet[..packet_len], &mut decoded)?;
        require(
            decoded_samples == OPUS_FRAME_SAMPLES,
            "Opus decoder returned an unexpected frame length",
        )?;
        total_packet_bytes += packet_len;
        packets.push(packet[..packet_len].to_vec());
        println!(
            "Opus frame {frame_index}: packet_bytes={packet_len}, decoded_samples_per_channel={decoded_samples}"
        );
    }

    println!(
        "Opus total: frames={OPUS_FRAMES}, channels={OPUS_CHANNELS}, encoded_bytes={total_packet_bytes}"
    );
    Ok(packets)
}

fn make_yuv420(frame_index: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let width = VIDEO_WIDTH as usize;
    let height = VIDEO_HEIGHT as usize;
    let chroma_width = width / 2;
    let chroma_height = height / 2;

    let mut y = vec![0_u8; width * height];
    for row in 0..height {
        for column in 0..width {
            y[row * width + column] = ((column * 3 + row * 5 + frame_index * 29) % 256) as u8;
        }
    }

    let mut u = vec![0_u8; chroma_width * chroma_height];
    let mut v = vec![0_u8; chroma_width * chroma_height];
    for row in 0..chroma_height {
        for column in 0..chroma_width {
            let index = row * chroma_width + column;
            u[index] = (96 + (column + frame_index * 3) % 48) as u8;
            v[index] = (160 + (row + frame_index * 5) % 48) as u8;
        }
    }

    (y, u, v)
}

fn run_vp9_probe() -> Result<Vec<Vp9Packet>, Box<dyn Error>> {
    let mut encoder = Vp9Encoder::new()?;
    let mut decoder = Vp9Decoder::new();
    let mut total_packet_bytes = 0_usize;
    let mut total_decoded_frames = 0_usize;
    let mut packets = Vec::with_capacity(VIDEO_FRAMES);

    for frame_index in 0..VIDEO_FRAMES {
        let (y, u, v) = make_yuv420(frame_index);
        let packet = encoder.encode_frame(Yuv420Frame {
            width: VIDEO_WIDTH,
            height: VIDEO_HEIGHT,
            planes: [&y, &u, &v],
            strides: [
                VIDEO_WIDTH as usize,
                (VIDEO_WIDTH / 2) as usize,
                (VIDEO_WIDTH / 2) as usize,
            ],
        })?;
        require(!packet.is_empty(), "VP9 encoder returned an empty packet")?;
        require(
            packet.len() <= MAX_VIDEO_PACKET_BYTES,
            "VP9 encoder returned an over-limit packet",
        )?;

        let pictures = decoder.decode_packet(&packet, Some(frame_index as i64))?;
        require(
            pictures.len() == 1,
            "low-latency VP9 probe did not decode exactly one displayed frame",
        )?;
        let picture = &pictures[0];
        require(
            (picture.width, picture.height) == (VIDEO_WIDTH, VIDEO_HEIGHT),
            "VP9 decoder returned unexpected dimensions",
        )?;
        require(
            picture.pts == Some(frame_index as i64),
            "VP9 decoder did not preserve the supplied timestamp",
        )?;
        require(
            picture.planes[0].len() == VIDEO_WIDTH as usize * VIDEO_HEIGHT as usize
                && picture.planes[1].len()
                    == (VIDEO_WIDTH as usize / 2) * (VIDEO_HEIGHT as usize / 2)
                && picture.planes[2].len()
                    == (VIDEO_WIDTH as usize / 2) * (VIDEO_HEIGHT as usize / 2),
            "VP9 decoder returned unexpected 4:2:0 plane lengths",
        )?;

        total_packet_bytes += packet.len();
        total_decoded_frames += pictures.len();
        println!(
            "VP9 frame {frame_index}: packet_bytes={}, decoded_frames={}, dimensions={}x{}",
            packet.len(),
            pictures.len(),
            picture.width,
            picture.height,
        );
        packets.push(Vp9Packet {
            timestamp_micros: frame_index as i64 * VIDEO_FRAME_DURATION_MICROS,
            chunk_type: vp9_chunk_type(&packet)?,
            data: packet,
        });
    }

    require(
        total_decoded_frames == VIDEO_FRAMES,
        "VP9 decoded-frame total did not match the input frame count",
    )?;
    println!(
        "VP9 total: frames={VIDEO_FRAMES}, decoded_frames={total_decoded_frames}, encoded_bytes={total_packet_bytes}"
    );
    Ok(packets)
}

fn read_vp9_bits(packet: &[u8], bit_offset: &mut usize, count: usize) -> Result<u8, io::Error> {
    let mut value = 0_u8;
    for _ in 0..count {
        let byte = packet
            .get(*bit_offset / 8)
            .ok_or_else(|| invalid_data("VP9 packet ended inside its uncompressed header"))?;
        let next_bit = (byte >> (7 - (*bit_offset % 8))) & 1;
        value = (value << 1) | next_bit;
        *bit_offset += 1;
    }
    Ok(value)
}

fn vp9_chunk_type(packet: &[u8]) -> Result<&'static str, io::Error> {
    let mut bit_offset = 0;
    let frame_marker = read_vp9_bits(packet, &mut bit_offset, 2)?;
    require(frame_marker == 2, "VP9 packet has an invalid frame marker")?;

    let profile_low = read_vp9_bits(packet, &mut bit_offset, 1)?;
    let profile_high = read_vp9_bits(packet, &mut bit_offset, 1)?;
    let profile = profile_low | (profile_high << 1);
    if profile == 3 {
        require(
            read_vp9_bits(packet, &mut bit_offset, 1)? == 0,
            "VP9 profile-3 header has a nonzero reserved bit",
        )?;
    }

    let show_existing_frame = read_vp9_bits(packet, &mut bit_offset, 1)?;
    if show_existing_frame != 0 {
        return Ok("delta");
    }

    let frame_type = read_vp9_bits(packet, &mut bit_offset, 1)?;
    Ok(if frame_type == 0 { "key" } else { "delta" })
}

fn write_byte_array(output: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    output.write_all(b"[")?;
    for (index, byte) in bytes.iter().enumerate() {
        if index != 0 {
            output.write_all(b",")?;
        }
        write!(output, "{byte}")?;
    }
    output.write_all(b"]")
}

fn write_opus_json(directory: &Path, packets: &[Vec<u8>]) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let file = File::create(directory.join("opus-sequence.json"))?;
    let mut output = BufWriter::new(file);
    writeln!(
        output,
        "{{\"codec\":\"opus\",\"sampleRate\":{OPUS_SAMPLE_RATE},\"channels\":{OPUS_CHANNELS},\"frames\":["
    )?;
    for (index, packet) in packets.iter().enumerate() {
        if index != 0 {
            output.write_all(b",\n")?;
        }
        write!(
            output,
            "{{\"timestamp\":{},\"duration\":{OPUS_FRAME_DURATION_MICROS},\"type\":\"key\",\"data\":",
            index as i64 * OPUS_FRAME_DURATION_MICROS
        )?;
        write_byte_array(&mut output, packet)?;
        output.write_all(b"}")?;
    }
    output.write_all(b"]}\n")?;
    output.flush()
}

fn write_vp9_json(directory: &Path, packets: &[Vp9Packet]) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let file = File::create(directory.join("vp9-sequence.json"))?;
    let mut output = BufWriter::new(file);
    writeln!(
        output,
        "{{\"codec\":\"vp09.00.10.08\",\"width\":{VIDEO_WIDTH},\"height\":{VIDEO_HEIGHT},\"frames\":["
    )?;
    for (index, packet) in packets.iter().enumerate() {
        if index != 0 {
            output.write_all(b",\n")?;
        }
        write!(
            output,
            "{{\"timestamp\":{},\"duration\":{VIDEO_FRAME_DURATION_MICROS},\"type\":\"{}\",\"data\":",
            packet.timestamp_micros, packet.chunk_type
        )?;
        write_byte_array(&mut output, &packet.data)?;
        output.write_all(b"}")?;
    }
    output.write_all(b"]}\n")?;
    output.flush()
}

fn main() -> Result<(), Box<dyn Error>> {
    let export_dir = parse_export_dir()?;
    refuse_vp9_environment_overrides()?;
    println!("Running a deterministic self-roundtrip functional probe; this is not an interoperability test.");
    let opus_packets = run_opus_probe()?;
    let vp9_packets = run_vp9_probe()?;
    if let Some(directory) = export_dir {
        write_opus_json(&directory, &opus_packets)?;
        write_vp9_json(&directory, &vp9_packets)?;
        println!(
            "Exported {} Opus and {} VP9 synthetic packets to {}",
            opus_packets.len(),
            vp9_packets.len(),
            directory.display(),
        );
    }
    println!("Codec probe passed.");
    Ok(())
}
