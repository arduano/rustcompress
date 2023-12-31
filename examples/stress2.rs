use rustcompress::compressors::lzma::codecs::{
    header_codec::{LzmaHeader, LzmaHeaderProps},
    length_codec::MATCH_LEN_MAX,
    lzma_stream_codec::{
        encoders::{
            instructions_normal::LZMANormalInstructionPicker, match_finding::hc4::HC4MatchFinder,
            LZMAEncoderInput,
        },
        LZMACodecEncoder,
    },
    range_codec::RangeEncoder,
};

fn main() {
    let data_part = include_bytes!("/mnt/fat/Midis/tau2.5.9.mid");

    let mut data = Vec::new();
    for _ in 0..1 {
        data.extend_from_slice(data_part);
    }

    for i in 0..1 {
        dbg!(i);
        let header = LzmaHeader {
            dict_size: 0x4000,
            props: LzmaHeaderProps {
                lc: 3,
                lp: 0,
                pb: 2,
            },
            uncompressed_size: data.len() as u64,
        };

        let mut compressed = Vec::new();

        let mut rc = RangeEncoder::new(&mut compressed);
        let nice_len = 270;
        // let picker = LZMAFastInstructionPicker::new(nice_len);
        let picker = LZMANormalInstructionPicker::new(nice_len, header.props.pb as u32);
        let mut encoder = LZMACodecEncoder::new(
            header.dict_size,
            header.props.lc as u32,
            header.props.lp as u32,
            header.props.pb as u32,
            nice_len,
            picker,
        );

        let mut encoder_buffer = LZMAEncoderInput::new(
            HC4MatchFinder::new(header.dict_size, nice_len, MATCH_LEN_MAX as u32, 48),
            header.dict_size,
        );

        for _ in 0..header.dict_size {
            encoder_buffer.append_data(&[0]);
            encoder_buffer.increment_pos();
        }

        let mut written = 0;
        let mut passed = 0;
        while written < data.len() {
            let free_bytes = encoder_buffer.available_append_bytes();
            if free_bytes > 0 {
                let to_write = std::cmp::min(free_bytes, data.len() - written);
                encoder_buffer.append_data(&data[written..written + to_write]);
                written += to_write;
            }

            let forward_before = encoder_buffer.forward_bytes();

            encoder
                .encode_one_packet(&mut rc, &mut encoder_buffer)
                .unwrap();

            let forward_after = encoder_buffer.forward_bytes();
            let offset = forward_before - forward_after;
            passed += offset;
        }

        while encoder_buffer.forward_bytes() > 0 {
            let forward_before = encoder_buffer.forward_bytes();

            encoder
                .encode_one_packet(&mut rc, &mut encoder_buffer)
                .unwrap();

            let forward_after = encoder_buffer.forward_bytes();
            let offset = forward_before - forward_after;
            passed += offset;
        }

        rc.finish().unwrap();

        dbg!(compressed.len());
    }
}
