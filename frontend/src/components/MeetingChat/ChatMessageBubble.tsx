'use client';

import { ChatUIMessage } from '@/hooks/useMeetingChat';

interface ChatMessageBubbleProps {
  message: ChatUIMessage;
}

export function ChatMessageBubble({ message }: ChatMessageBubbleProps) {
  const isUser = message.role === 'user';

  return (
    <div className={`flex ${isUser ? 'justify-end' : 'justify-start'}`}>
      <div
        className={`max-w-[85%] rounded-2xl px-3 py-2 text-sm whitespace-pre-wrap break-words ${
          isUser
            ? 'bg-blue-600 text-white rounded-br-sm'
            : 'bg-gray-100 text-gray-900 rounded-bl-sm'
        }`}
      >
        {message.content}
        {message.isStreaming && (
          <span className="inline-block w-2 h-4 ml-0.5 align-text-bottom bg-gray-400 animate-pulse rounded-sm" />
        )}
      </div>
    </div>
  );
}
