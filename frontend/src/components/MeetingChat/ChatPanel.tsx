'use client';

import { useEffect, useRef } from 'react';
import { Button } from '@/components/ui/button';
import { MessageSquare, Trash2, X } from 'lucide-react';
import { useMeetingChat } from '@/hooks/useMeetingChat';
import { ChatMessageBubble } from './ChatMessageBubble';
import { ChatInput } from './ChatInput';

interface ChatPanelProps {
  meetingId: string | null;
  /** Returns the live transcript text at send time; return '' for saved
   *  meetings so the backend loads the stored transcript. */
  getTranscriptText: () => string;
  /** Whether any transcript exists yet (controls the disabled state). */
  hasTranscript: boolean;
  onClose?: () => void;
}

export function ChatPanel({
  meetingId,
  getTranscriptText,
  hasTranscript,
  onClose,
}: ChatPanelProps) {
  const { messages, isStreaming, error, sendMessage, cancel, clearHistory } =
    useMeetingChat(meetingId, getTranscriptText);

  const scrollRef = useRef<HTMLDivElement>(null);
  const isAtBottomRef = useRef(true);

  // Auto-scroll to bottom on new content unless the user scrolled up
  useEffect(() => {
    const el = scrollRef.current;
    if (el && isAtBottomRef.current) {
      el.scrollTop = el.scrollHeight;
    }
  }, [messages]);

  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    isAtBottomRef.current =
      el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  };

  const disabled = !meetingId || !hasTranscript;
  const placeholder = !meetingId
    ? 'Start recording to chat'
    : !hasTranscript
      ? 'Waiting for transcript...'
      : 'Ask about this meeting...';

  return (
    <div className="flex flex-col h-full bg-white border-l border-gray-200">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200">
        <div className="flex items-center gap-2 text-sm font-medium text-gray-700">
          <MessageSquare className="h-4 w-4" />
          Chat with transcript
        </div>
        <div className="flex items-center gap-1">
          {messages.length > 0 && (
            <Button
              variant="ghost"
              size="icon"
              onClick={clearHistory}
              disabled={isStreaming}
              title="Clear chat history"
              className="h-7 w-7"
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          )}
          {onClose && (
            <Button
              variant="ghost"
              size="icon"
              onClick={onClose}
              title="Close chat"
              className="h-7 w-7"
            >
              <X className="h-4 w-4" />
            </Button>
          )}
        </div>
      </div>

      {/* Message list */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto px-3 py-3 space-y-3"
      >
        {messages.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center text-gray-400 text-sm px-4">
            <MessageSquare className="h-8 w-8 mb-2" />
            <p>Ask a question about this meeting.</p>
            <p className="mt-1 text-xs">
              The answer uses the transcript so far — you can ask while the
              meeting is still in progress.
            </p>
          </div>
        ) : (
          messages.map((message) => (
            <ChatMessageBubble key={message.id} message={message} />
          ))
        )}
      </div>

      {/* Error */}
      {error && (
        <div className="px-3 py-2 text-xs text-red-600 bg-red-50 border-t border-red-100">
          {error}
        </div>
      )}

      {/* Input */}
      <ChatInput
        disabled={disabled}
        isStreaming={isStreaming}
        placeholder={placeholder}
        onSend={sendMessage}
        onCancel={cancel}
      />
    </div>
  );
}
