/**
 * Chat Service
 *
 * Handles all "chat with transcript" Tauri backend calls.
 * Pure 1-to-1 wrapper - no error handling changes, exact same behavior as direct invoke calls.
 */

import { invoke } from '@tauri-apps/api/core';

export interface ChatMessage {
  id: string;
  meeting_id: string;
  role: 'user' | 'assistant';
  content: string;
  created_at: string;
}

export interface SendChatMessageResponse {
  user_message_id: string;
  assistant_message_id: string;
}

export interface ChatStreamChunk {
  meeting_id: string;
  message_id: string;
  delta: string;
  done: boolean;
  error: string | null;
}

export class ChatService {
  /**
   * Send a user message and start streaming the assistant response.
   * Pass transcriptText for live recordings; pass '' for saved meetings
   * (the backend loads the saved transcript itself).
   * The response streams via 'chat-stream-chunk' Tauri events.
   */
  async sendMessage(
    meetingId: string,
    message: string,
    transcriptText: string
  ): Promise<SendChatMessageResponse> {
    return invoke<SendChatMessageResponse>('api_send_chat_message', {
      meetingId,
      message,
      transcriptText,
    });
  }

  /**
   * Cancel the in-progress chat stream for a meeting.
   */
  async cancelChat(meetingId: string): Promise<{ cancelled: boolean }> {
    return invoke<{ cancelled: boolean }>('api_cancel_chat', { meetingId });
  }

  /**
   * Get the persisted chat history for a meeting in chronological order.
   */
  async getChatHistory(meetingId: string): Promise<ChatMessage[]> {
    return invoke<ChatMessage[]>('api_get_chat_history', { meetingId });
  }

  /**
   * Delete all chat messages for a meeting.
   */
  async clearChatHistory(meetingId: string): Promise<{ deleted: number }> {
    return invoke<{ deleted: number }>('api_clear_chat_history', { meetingId });
  }

  /**
   * Re-key chat messages from the provisional live-recording meeting id to
   * the final id assigned when the meeting was saved.
   */
  async reassignChatHistory(
    oldMeetingId: string,
    newMeetingId: string
  ): Promise<{ updated: number }> {
    return invoke<{ updated: number }>('api_reassign_chat_history', {
      oldMeetingId,
      newMeetingId,
    });
  }
}

// Export singleton instance
export const chatService = new ChatService();
