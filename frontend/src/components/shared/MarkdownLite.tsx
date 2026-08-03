'use client';

import React from 'react';

// Minimal read-only markdown renderer for agent outputs. The summary panel's
// BlockNote editor is an editing surface and far too heavy for read-only
// display, and the repo has no other markdown renderer, so this covers the
// subset agents emit: headings, bullet/numbered lists, checkboxes, bold,
// italics, and inline code. Everything is rendered as React elements (no
// injected HTML).

function renderInline(text: string, keyPrefix: string): React.ReactNode[] {
  // Split on **bold**, *italic*, and `code` spans.
  const parts = text.split(/(\*\*[^*]+\*\*|\*[^*]+\*|`[^`]+`)/g);
  return parts.map((part, index) => {
    const key = `${keyPrefix}-${index}`;
    if (part.startsWith('**') && part.endsWith('**') && part.length > 4) {
      return <strong key={key}>{part.slice(2, -2)}</strong>;
    }
    if (part.startsWith('*') && part.endsWith('*') && part.length > 2) {
      return <em key={key}>{part.slice(1, -1)}</em>;
    }
    if (part.startsWith('`') && part.endsWith('`') && part.length > 2) {
      return (
        <code key={key} className="px-1 py-0.5 bg-gray-100 rounded text-[0.9em]">
          {part.slice(1, -1)}
        </code>
      );
    }
    return <React.Fragment key={key}>{part}</React.Fragment>;
  });
}

interface Line {
  raw: string;
}

export function MarkdownLite({ markdown }: { markdown: string }) {
  const lines: Line[] = markdown.replace(/\r\n/g, '\n').split('\n').map(raw => ({ raw }));
  const blocks: React.ReactNode[] = [];
  let listBuffer: React.ReactNode[] = [];
  let listOrdered = false;

  const flushList = (key: string) => {
    if (!listBuffer.length) return;
    const items = listBuffer;
    listBuffer = [];
    blocks.push(
      listOrdered ? (
        <ol key={key} className="list-decimal pl-5 space-y-1">{items}</ol>
      ) : (
        <ul key={key} className="list-disc pl-5 space-y-1">{items}</ul>
      )
    );
  };

  lines.forEach((line, index) => {
    const raw = line.raw;
    const trimmed = raw.trim();
    const key = `md-${index}`;

    const heading = /^(#{1,4})\s+(.*)$/.exec(trimmed);
    const bullet = /^[-*]\s+(.*)$/.exec(trimmed);
    const ordered = /^\d+[.)]\s+(.*)$/.exec(trimmed);

    if (heading) {
      flushList(`${key}-flush`);
      const level = heading[1].length;
      const content = renderInline(heading[2], key);
      const classes = [
        'text-lg font-semibold text-gray-900 mt-2',
        'text-base font-semibold text-gray-900 mt-2',
        'text-sm font-semibold text-gray-800 mt-1',
        'text-sm font-medium text-gray-800 mt-1',
      ][level - 1];
      blocks.push(<div key={key} className={classes}>{content}</div>);
      return;
    }

    if (bullet || ordered) {
      const body = (bullet ? bullet[1] : ordered![1]);
      const nextOrdered = !!ordered;
      if (listBuffer.length && listOrdered !== nextOrdered) {
        flushList(`${key}-switch`);
      }
      listOrdered = nextOrdered;

      const checkbox = /^\[([ xX])\]\s+(.*)$/.exec(body);
      if (checkbox) {
        listBuffer.push(
          <li key={key} className="list-none -ml-5 flex items-start gap-2">
            <input type="checkbox" checked={checkbox[1] !== ' '} readOnly className="mt-1" />
            <span>{renderInline(checkbox[2], key)}</span>
          </li>
        );
      } else {
        listBuffer.push(<li key={key}>{renderInline(body, key)}</li>);
      }
      return;
    }

    flushList(`${key}-flush`);
    if (trimmed.length) {
      blocks.push(<p key={key}>{renderInline(trimmed, key)}</p>);
    }
  });

  flushList('md-tail');

  return <div className="space-y-2 text-sm text-gray-700 leading-relaxed">{blocks}</div>;
}
