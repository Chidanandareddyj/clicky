//! Deepgram Listen (async) consumed from a spawned Tokio runtime on a worker thread.

use futures_util::{SinkExt, StreamExt};
use http::HeaderValue;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::Message},
};

fn deepgram_listen_token_url(proxy_base_trimmed: &str) -> String {
    format!(
        "{}/deepgram-listen-token",
        proxy_base_trimmed.trim_end_matches('/')
    )
}

async fn fetch_listen_token_async(proxy_base_trimmed: &str) -> Result<String, String> {
    let http_client_snapshot = reqwest::Client::new();
    let response_from_worker = http_client_snapshot
        .post(deepgram_listen_token_url(proxy_base_trimmed))
        .body("{}")
        .header("content-type", "application/json")
        .send()
        .await
        .map_err(|network_error| network_error.to_string())?;
    if !response_from_worker.status().is_success() {
        let failure_status_code_measurement_inside = response_from_worker.status();
        let failure_body_measurement_text_inside = response_from_worker
            .text()
            .await
            .unwrap_or_else(|_| String::new());
        return Err(format!(
            "{failure_status_code_measurement_inside}: {failure_body_measurement_text_inside}"
        ));
    }
    let json_body: Value = response_from_worker
        .json()
        .await
        .map_err(|e| e.to_string())?;
    json_body
        .get("access_token")
        .and_then(|jwt_token_maybe| jwt_token_maybe.as_str())
        .map(std::string::ToString::to_string)
        .ok_or_else(|| "missing access_token in token response".to_string())
}

fn parse_deepgram_sentence(value_body: &Value) -> Option<(String, bool)> {
    let sentence_text_maybe = value_body
        .pointer("/channel/alternatives/0/transcript")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if sentence_text_maybe.trim().is_empty() {
        return None;
    }
    let is_sentence_final_marker = value_body
        .get("is_final")
        .and_then(Value::as_bool)
        .or_else(|| value_body.get("speech_final").and_then(Value::as_bool))
        .unwrap_or(false);
    Some((sentence_text_maybe, is_sentence_final_marker))
}

pub async fn run_deepgram_listen_session(
    app_handle_sender: AppHandle,
    proxy_base_url_string: String,
    mut pcm_incoming_receiver: tokio::sync::mpsc::UnboundedReceiver<Vec<i16>>,
    mut cancel_listen_receiver: tokio::sync::watch::Receiver<bool>,
) -> Result<String, String> {
    let deepgram_listen_access_token = fetch_listen_token_async(&proxy_base_url_string).await?;

    let listen_websocket_url_qs = "wss://api.deepgram.com/v1/listen?model=nova-2&encoding=linear16&sample_rate=16000&channels=1&interim_results=true&smart_format=true&punctuate=true";
    let mut handshake_listen_request_document = listen_websocket_url_qs
        .into_client_request()
        .map_err(|request_error_message| request_error_message.to_string())?;
    handshake_listen_request_document.headers_mut().insert(
        http::header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {deepgram_listen_access_token}"))
            .map_err(|err| err.to_string())?,
    );

    let (websocket_full_duplex_sender, websocket_full_duplex_receiver) =
        connect_async(handshake_listen_request_document)
            .await
            .map_err(|handshake_problem| handshake_problem.to_string())?
            .0
            .split();

    let mut websocket_write_half = websocket_full_duplex_sender;
    tokio::pin!(websocket_full_duplex_receiver);

    let mut finalized_sentence_collect: Vec<String> = Vec::new();
    let mut last_partial_sentence_cache = String::new();

    loop {
        tokio::select! {

            biased;

            _ignored_cancel_signal = cancel_listen_receiver.changed() => {
                if *cancel_listen_receiver.borrow() {
                    break;
                }
            }

            maybe_pcm_take = pcm_incoming_receiver.recv() => {
                match maybe_pcm_take {
                    Some(pcm_take_vector) => {
                        let mut little_endian_encoded_audio = Vec::with_capacity(pcm_take_vector.len() * 2);
                        for one_sample_piece in pcm_take_vector {
                            little_endian_encoded_audio.extend_from_slice(&one_sample_piece.to_le_bytes());
                        }
                        websocket_write_half
                            .send(Message::Binary(little_endian_encoded_audio))
                            .await
                            .map_err(|socket_write_error| socket_write_error.to_string())?;
                    }
                    None => break,
                }
            }

            maybe_socket_message = websocket_full_duplex_receiver.next() => {
                match maybe_socket_message {
                    Some(Ok(Message::Text(message_text_payload_received))) => {
                        let serde_json_maybe: Value = serde_json::from_str(&message_text_payload_received)
                            .unwrap_or(Value::Null);
                        let categorized_type_string =
                            serde_json_maybe.get("type").and_then(Value::as_str).unwrap_or("");
                        if categorized_type_string == "Error" {
                            return Err(serde_json_maybe.to_string());
                        }
                        if let Some((transcript_maybe, final_marker)) =
                            parse_deepgram_sentence(&serde_json_maybe)
                        {
                            if final_marker {
                                finalized_sentence_collect.push(transcript_maybe.clone());
                                let _ignored_emit_sentence_final_piece = app_handle_sender.emit(
                                    "stt-final",
                                    serde_json::json!( { "text": transcript_maybe }),
                                );
                            } else if transcript_maybe != last_partial_sentence_cache {
                                last_partial_sentence_cache.clone_from(&transcript_maybe);
                                let _ignored_emit_partial_sentence_piece = app_handle_sender.emit(
                                    "stt-partial",
                                    serde_json::json!( { "text": transcript_maybe }),
                                );
                            }
                        }
                    }
                    Some(Ok(Message::Close(_close_reason))) => break,
                    Some(Err(socket_read_problem)) => return Err(socket_read_problem.to_string()),
                    Some(Ok(_other_ping_pong_ping)) => {}
                    None => break,
                }
            }
        }
    }

    let _ignored_close_listen_socket_maybe = websocket_write_half.send(Message::Close(None)).await;

    while let Some(late_message_arrival) = websocket_full_duplex_receiver.next().await {
        match late_message_arrival {
            Ok(Message::Text(tail_text_chunk)) => {
                let serde_json_tail: Value =
                    serde_json::from_str(&tail_text_chunk).unwrap_or(Value::Null);
                if let Some((transcript_maybe_tail, finalized_tail)) =
                    parse_deepgram_sentence(&serde_json_tail)
                {
                    if finalized_tail {
                        finalized_sentence_collect.push(transcript_maybe_tail);
                    }
                }
            }
            _ => {}
        }
    }

    let joined_final_user_utterance = finalized_sentence_collect.join(" ").trim().to_string();
    Ok(joined_final_user_utterance)
}
