'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import {
  chatService,
  ChatMessage,
  ChatStreamChunk,
} from '@/services/chatService';

export interface ChatUIMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  isStreaming?: boolean;
}

/**
 * Chat-with-transcript state for one meeting.
 *
 * @param meetingId - current meeting id (provisional live id or saved id)
 * @param getTranscriptText - returns the live transcript text at send time;
 *   return '' for saved meetings so the backend loads the stored transcript.
 */
export function useMeetingChat(
  meetingId: string | null,
  getTranscriptText: () => string
) {
  const [messages, setMessages] = useState<ChatUIMessage[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const meetingIdRef = useRef(meetingId);
  meetingIdRef.current = meetingId;

  // Load persisted history whenever the meeting changes
  useEffect(() => {
    setMessages([]);
    setError(null);
    setIsStreaming(false);

    if (!meetingId) return;

    let cancelled = false;
    chatService
      .getChatHistory(meetingId)
      .then((history) => {
        if (cancelled) return;
        setMessages(
          history
            .filter((m) => m.content.trim() !== '')
            .map((m) => ({ id: m.id, role: m.role, content: m.content }))
        );
      })
      .catch((err) => {
        console.warn('Failed to load chat history:', err);
      });

    return () => {
      cancelled = true;
    };
  }, [meetingId]);

  // Single stream-event subscription, filtered to the current meeting
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let disposed = false;

    // Stream chunks can arrive before the send command resolves with the real
    // message ids; adopt the real id onto the optimistic placeholder so no
    // deltas are dropped.
    const applyToTarget = (
      prev: ChatUIMessage[],
      messageId: string,
      update: (m: ChatUIMessage) => ChatUIMessage
    ): ChatUIMessage[] => {
      if (prev.some((m) => m.id === messageId)) {
        return prev.map((m) => (m.id === messageId ? update(m) : m));
      }
      // Adopt onto the most recent optimistic assistant placeholder
      for (let i = prev.length - 1; i >= 0; i--) {
        if (prev[i].role === 'assistant' && prev[i].id.startsWith('temp-assistant-')) {
          const next = [...prev];
          next[i] = update({ ...next[i], id: messageId });
          return next;
        }
      }
      return prev;
    };

    listen<ChatStreamChunk>('chat-stream-chunk', (event) => {
      const chunk = event.payload;
      if (chunk.meeting_id !== meetingIdRef.current) return;

      if (chunk.done) {
        setIsStreaming(false);
        if (chunk.error && chunk.error !== 'cancelled') {
          setError(chunk.error);
          // The backend persisted the error note into the message; reflect it
          setMessages((prev) =>
            applyToTarget(prev, chunk.message_id, (m) => ({
              ...m,
              content: m.content || `⚠️ ${chunk.error}`,
              isStreaming: false,
            }))
          );
        } else {
          setMessages((prev) =>
            applyToTarget(prev, chunk.message_id, (m) => ({
              ...m,
              content:
                chunk.error === 'cancelled'
                  ? `${m.content}\n\n_[response cancelled]_`.trimStart()
                  : m.content,
              isStreaming: false,
            }))
          );
        }
        return;
      }

      setMessages((prev) =>
        applyToTarget(prev, chunk.message_id, (m) => ({
          ...m,
          content: m.content + chunk.delta,
        }))
      );
    }).then((fn) => {
      if (disposed) {
        fn();
      } else {
        unlisten = fn;
      }
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const sendMessage = useCallback(
    async (text: string) => {
      const trimmed = text.trim();
      if (!trimmed || !meetingId || isStreaming) return;

      setError(null);
      setIsStreaming(true);

      // Optimistic placeholders; ids reconciled from the command response
      const tempUserId = `temp-user-${Date.now()}`;
      const tempAssistantId = `temp-assistant-${Date.now()}`;
      setMessages((prev) => [
        ...prev,
        { id: tempUserId, role: 'user', content: trimmed },
        { id: tempAssistantId, role: 'assistant', content: '', isStreaming: true },
      ]);

      try {
        const response = await chatService.sendMessage(
          meetingId,
          trimmed,
          getTranscriptText()
        );
        setMessages((prev) =>
          prev.map((m) => {
            if (m.id === tempUserId) return { ...m, id: response.user_message_id };
            if (m.id === tempAssistantId)
              return { ...m, id: response.assistant_message_id };
            return m;
          })
        );
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setIsStreaming(false);
        // Drop the optimistic placeholders on failure
        setMessages((prev) =>
          prev.filter((m) => m.id !== tempUserId && m.id !== tempAssistantId)
        );
      }
    },
    [meetingId, isStreaming, getTranscriptText]
  );

  const cancel = useCallback(async () => {
    if (!meetingId) return;
    try {
      await chatService.cancelChat(meetingId);
    } catch (err) {
      console.warn('Failed to cancel chat:', err);
    }
  }, [meetingId]);

  const clearHistory = useCallback(async () => {
    if (!meetingId || isStreaming) return;
    try {
      await chatService.clearChatHistory(meetingId);
      setMessages([]);
      setError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    }
  }, [meetingId, isStreaming]);

  return { messages, isStreaming, error, sendMessage, cancel, clearHistory };
}
