/// Chat module - "chat with the transcript" feature
///
/// Lets the user ask questions about a meeting transcript, both while the
/// meeting is still being recorded (transcript text supplied by the frontend)
/// and for completed meetings (transcript loaded from the database).
///
/// Responses stream token-by-token to the frontend via the
/// `chat-stream-chunk` Tauri event. Chat history persists in the
/// `chat_messages` table.
pub mod commands;
pub mod prompt;
pub mod stream;
