'use client';

import { useState, KeyboardEvent } from 'react';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { Send, Square } from 'lucide-react';

interface ChatInputProps {
  disabled: boolean;
  isStreaming: boolean;
  placeholder: string;
  onSend: (text: string) => void;
  onCancel: () => void;
}

export function ChatInput({
  disabled,
  isStreaming,
  placeholder,
  onSend,
  onCancel,
}: ChatInputProps) {
  const [text, setText] = useState('');

  const handleSend = () => {
    const trimmed = text.trim();
    if (!trimmed || disabled || isStreaming) return;
    onSend(trimmed);
    setText('');
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  return (
    <div className="flex items-end gap-2 border-t border-gray-200 p-3 bg-white">
      <Textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder={placeholder}
        disabled={disabled}
        rows={1}
        className="min-h-[38px] max-h-32 resize-none text-sm"
      />
      {isStreaming ? (
        <Button
          variant="outline"
          size="icon"
          onClick={onCancel}
          title="Stop response"
          className="shrink-0"
        >
          <Square className="h-4 w-4" />
        </Button>
      ) : (
        <Button
          size="icon"
          onClick={handleSend}
          disabled={disabled || !text.trim()}
          title="Send message"
          className="shrink-0"
        >
          <Send className="h-4 w-4" />
        </Button>
      )}
    </div>
  );
}
