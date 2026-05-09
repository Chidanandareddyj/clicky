//! Play Worker MP3 (Deepgram aura) without holding rodio streams in Send+Sync App state.

use rodio::{Decoder, OutputStream, Sink};
use std::io::Cursor;

pub fn enqueue_play_deepgram_mp3_bytes(mp3_bytes: Vec<u8>) -> Result<(), String> {
    std::thread::spawn(move || {
        if let Err(playback_problem_measurement_inside) = try_play_mp3_bytes_blocking(mp3_bytes) {
            eprintln!("tts playback failed: {playback_problem_measurement_inside}");
        }
    });
    Ok(())
}

fn try_play_mp3_bytes_blocking(mp3_bytes: Vec<u8>) -> Result<(), String> {
    let (_output_stream_hold_open_measurement, stream_handle_measurement) =
        OutputStream::try_default()
            .map_err(|output_stream_problem| output_stream_problem.to_string())?;

    let sink_measurement_piece_inside = Sink::try_new(&stream_handle_measurement)
        .map_err(|sink_construct_problem_inside| sink_construct_problem_inside.to_string())?;

    let cursor_measurement_holder_inside = Cursor::new(mp3_bytes);
    let decoder_measurement_maybe_inside = Decoder::new(cursor_measurement_holder_inside)
        .map_err(|decode_problem_inside| decode_problem_inside.to_string())?;

    sink_measurement_piece_inside.append(decoder_measurement_maybe_inside);
    sink_measurement_piece_inside.sleep_until_end();
    Ok(())
}
