// Tidying Markdown pasted from chat assistants, web pages, word processors and notes apps.

import { gfm } from "@joplin/turndown-plugin-gfm";
import DOMPurify from "dompurify";
import { Marked, type Tokens } from "marked";
import TurndownService from "turndown";

export interface Cleaned {
  text: string;
  /** What was changed, in words, such as "3 bullets made into dashes". */
  fixes: string[];
}

/** A fence that opens or closes a code block: three or more backticks or tildes, indented at most three spaces. */
const FENCE = /^ {0,3}(`{3,}|~{3,})/;
/** Characters that take no room and only cause trouble: zero-width space, word joiner, byte order mark. */
const INVISIBLE = /[​⁠﻿]/g;
/** Spaces that don't break, which look like spaces but stop Markdown from seeing indentation and markers. */
const ODD_SPACE = /[   ]/g;
/** Bullets that word processors and web pages use, at the start of a line and followed by a space. */
const BULLET = /^(\s*)[•◦▪▫●○■□‣⁃∙·–]\s+/;
const OTHER_MARKER = /^(\s*)[*+](\s+)(?=\S)/;
const HEADING = /^ {0,3}#{1,6}(\s|$)/;
/** A heading without its space: "##Title", or "#Title" when a capital follows (so "#hashtag" stays). */
const TIGHT_HEADING = /^( {0,3})(#{2,6}|#(?=\p{Lu}))(?=[^\s#])/u;
const SMART_QUOTES: [RegExp, string][] = [
  [/[“”„‟″]/g, '"'],
  [/[‘’‚‛′]/g, "'"],
];

/**
 * Tidies Markdown without changing what it says: line endings, invisible characters, odd spaces, trailing spaces,
 * bullet characters, headings without a space or blank lines around them, runs of blank lines, curly quotes in code
 * and code blocks that were never closed. What is inside code blocks is otherwise left alone.
 */
export function cleanMarkdown(input: string): Cleaned {
  const counts = new Map<string, number>();
  const bump = (fix: string, n = 1) => n > 0 && counts.set(fix, (counts.get(fix) ?? 0) + n);

  let text = input;
  if (/\r/.test(text)) {
    text = text.replace(/\r\n?/g, "\n");
    bump("Windows line endings made plain");
  }
  const invisible = text.match(INVISIBLE)?.length ?? 0;
  if (invisible) {
    text = text.replace(INVISIBLE, "");
    bump("invisible characters removed", invisible);
  }

  const lines = text.split("\n");
  const out: string[] = [];
  // The fence that opened the code block we are in, if any.
  let fence: { char: string; length: number } | null = null;
  const lastIsBlank = () => out.length === 0 || out[out.length - 1].trim() === "";

  for (let i = 0; i < lines.length; i++) {
    let line = lines[i];
    const opener = line.match(FENCE);

    if (fence) {
      // Inside a code block: only curly quotes, which break code, are put right.
      if (opener && opener[1][0] === fence.char && opener[1].length >= fence.length && !line.slice(opener[0].length).trim()) {
        fence = null;
        out.push(line.trimEnd());
        continue;
      }
      let quotes = 0;
      for (const [pattern, plain] of SMART_QUOTES) {
        line = line.replace(pattern, () => {
          quotes++;
          return plain;
        });
      }
      bump("curly quotes in code made straight", quotes);
      out.push(line);
      continue;
    }

    const odd = line.match(ODD_SPACE)?.length ?? 0;
    if (odd) {
      const n = odd;
      line = line.replace(ODD_SPACE, " ");
      bump("non-breaking spaces made plain", n);
    }

    // Trailing spaces, except two or more before a line that goes on, which is a deliberate line break.
    const trimmed = line.trimEnd();
    if (trimmed !== line) {
      const next = lines[i + 1];
      const hardBreak = /\S {2,}$/.test(line) && next !== undefined && next.trim() !== "";
      const kept = hardBreak ? `${trimmed}  ` : trimmed;
      if (kept !== line) bump("lines with trailing spaces trimmed");
      line = kept;
    }

    if (opener) {
      fence = { char: opener[1][0], length: opener[1].length };
      out.push(line);
      continue;
    }

    const bullet = line.match(BULLET);
    if (bullet) {
      line = `${bullet[1]}- ${line.slice(bullet[0].length)}`;
      bump("bullets made into dashes");
    } else {
      const marker = line.match(OTHER_MARKER);
      // "* " and "+ " lists, but not "**bold**" or a horizontal rule of stars.
      if (marker && !/^\s*\*\s*\*\s*\*[\s*]*$/.test(line)) {
        line = `${marker[1]}- ${line.slice(marker[0].length)}`;
        bump("bullets made into dashes");
      }
    }

    const tight = line.match(TIGHT_HEADING);
    if (tight) {
      line = `${tight[1]}${tight[2]} ${line.slice(tight[0].length)}`;
      bump("headings given their space");
    }

    const blank = line.trim() === "";
    if (blank) {
      // One blank line between blocks is enough; none at the start.
      if (lastIsBlank()) {
        bump("extra blank lines removed");
        continue;
      }
      out.push("");
      continue;
    }

    if (HEADING.test(line)) {
      if (!lastIsBlank()) {
        out.push("");
        bump("blank lines added around headings");
      }
      out.push(line);
      const next = lines[i + 1];
      if (next !== undefined && next.trim() !== "") {
        out.push("");
        bump("blank lines added around headings");
      }
      continue;
    }

    out.push(line);
  }

  if (fence) {
    if (out[out.length - 1]?.trim() === "") out.pop();
    out.push(fence.char.repeat(fence.length));
    bump("unclosed code block closed");
  }
  while (out.length && out[out.length - 1].trim() === "") out.pop();
  const result = out.length ? `${out.join("\n")}\n` : "";

  const fixes = [...counts].map(([fix, n]) => (n > 1 && !/^(Windows|unclosed)/.test(fix) ? `${n} ${fix}` : fix));
  return { text: result, fixes: result === input ? [] : fixes.length ? fixes : ["whitespace tidied"] };
}

const LIST_ITEM = /^\s*([-*+]|\d{1,9}[.)])(\s+|$)/;
/** Lines that start a block of their own, which are never joined onto the line before. */
const BLOCK_START = /^\s*(#{1,6}(\s|$)|>|\||<|([-*_]\s*){3,}$|([-*+]|\d{1,9}[.)])(\s|$))/;
const leading = (line: string) => line.match(/^[ \t]*/)![0];
const width = (indent: string) => [...indent].reduce((n, c) => n + (c === "\t" ? 4 : 1), 0);

/** The line with `columns` of its indentation removed, a tab counting as four. */
function removeColumns(line: string, columns: number): string {
  let i = 0;
  let removed = 0;
  while (removed < columns && (line[i] === " " || line[i] === "\t")) {
    const w = line[i] === "\t" ? 4 : 1;
    if (removed + w > columns) return " ".repeat(removed + w - columns) + line.slice(i + 1);
    removed += w;
    i++;
  }
  return line.slice(i);
}

/** Calls `each` with the lines outside code blocks; the fence lines themselves count as outside. */
function outsideCode(lines: string[], each: (line: string, i: number) => void) {
  let fence: { char: string; length: number } | null = null;
  lines.forEach((line, i) => {
    const opener = line.match(FENCE);
    if (fence) {
      if (opener && opener[1][0] === fence.char && opener[1].length >= fence.length) {
        fence = null;
        each(line, i);
      }
      return;
    }
    if (opener) fence = { char: opener[1][0], length: opener[1].length };
    each(line, i);
  });
}

/**
 * Brings indented text back to the left edge, where four spaces or a tab would otherwise make it a code block.
 * Nested list items and the text that belongs to a list item keep their indentation, as do code blocks' contents.
 */
export function alignLeft(input: string): Cleaned {
  const lines = input.replace(/\r\n?/g, "\n").split("\n");
  let moved = 0;
  // The indentation of the list above's first-level items, if there is a list: lines indented further are nested in
  // it and move left by as much as its items do, so the nesting stays.
  let list: number | null = null;
  let blankBefore = true;
  outsideCode(lines, (line, i) => {
    if (!line.trim()) {
      blankBefore = true;
      return;
    }
    const indent = width(leading(line));
    const isItem = LIST_ITEM.test(line);
    const nested = list !== null && (indent >= list + 2 || (!blankBefore && !isItem && !BLOCK_START.test(line)));
    blankBefore = false;
    let by = indent;
    if (nested) by = Math.min(indent, list!);
    else list = isItem ? indent : null;
    if (!by) return;
    lines[i] = removeColumns(line, by);
    moved++;
  });
  const text = lines.join("\n");
  return { text, fixes: moved ? [`${moved} ${moved === 1 ? "line" : "lines"} moved to the left edge`] : [] };
}

/**
 * Joins lines that were broken in the middle of a sentence, as text copied from PDFs, emails and terminals often is,
 * so each paragraph and list item is one line. Line breaks made on purpose (two spaces or a backslash at the end),
 * headings, quotes, tables and code are left as they are.
 */
export function joinLines(input: string): Cleaned {
  const lines = input.replace(/\r\n?/g, "\n").split("\n");
  const outside = new Set<number>();
  outsideCode(lines, (line, i) => !FENCE.test(line) && outside.add(i));
  const out: string[] = [];
  let joined = 0;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const prev = out.length - 1;
    const canJoin =
      outside.has(i) &&
      outside.has(i - 1) &&
      line.trim() !== "" &&
      !BLOCK_START.test(line) &&
      !line.includes("|") &&
      prev >= 0 &&
      out[prev].trim() !== "" &&
      !/^\s*(#{1,6}\s|>|\||<)/.test(out[prev]) &&
      !out[prev].includes("|") &&
      !/( {2,}|\\)$/.test(out[prev]);
    if (canJoin) {
      out[prev] = `${out[prev].trimEnd()} ${line.trim()}`;
      joined++;
    } else out.push(line);
  }
  return {
    text: out.join("\n"),
    fixes: joined ? [`${joined} broken ${joined === 1 ? "line" : "lines"} joined`] : [],
  };
}

const marked = new Marked({ gfm: true, breaks: false });

const sanitize = (html: string) =>
  DOMPurify.sanitize(html, { USE_PROFILES: { html: true }, FORBID_TAGS: ["style", "form", "input"] });

/** Markdown as HTML that is safe to show: scripts, event handlers and the like are removed. */
export function renderMarkdown(text: string): string {
  return sanitize(marked.parse(text, { async: false }));
}

/** One block of a document (a paragraph, heading, list, table, code block…) and the source it came from. */
export interface Block {
  /** The block's own Markdown. */
  raw: string;
  /** What follows it up to the next block: blank lines and link definitions. */
  tail: string;
  /** Where its Markdown starts and ends in the document. */
  from: number;
  to: number;
  html: string;
}

export interface Blocks {
  /** Blank lines and link definitions before the first block. */
  lead: string;
  blocks: Block[];
}

/** The document rendered a block at a time, so the preview can tell which part of the source each part shows. */
export function renderBlocks(text: string): Blocks {
  const tokens = marked.lexer(text);
  const result: Blocks = { lead: "", blocks: [] };
  let pos = 0;
  for (const token of tokens) {
    const from = pos;
    pos += token.raw.length;
    const last = result.blocks[result.blocks.length - 1];
    if (token.type === "space" || token.type === "def") {
      if (last) last.tail += token.raw;
      else result.lead += token.raw;
      continue;
    }
    const list = Object.assign([token], { links: tokens.links }) as Tokens.Generic[] & { links: typeof tokens.links };
    result.blocks.push({ raw: token.raw, tail: "", from, to: pos, html: sanitize(marked.parser(list)) });
  }
  return result;
}

const turndown = new TurndownService({
  headingStyle: "atx",
  bulletListMarker: "-",
  codeBlockStyle: "fenced",
  emDelimiter: "*",
  strongDelimiter: "**",
  hr: "---",
});
turndown.use(gfm);
// List items with one space after the marker, as people write them, rather than turndown's three.
turndown.addRule("listItem", {
  filter: "li",
  replacement(content, node) {
    const parent = node.parentNode as HTMLElement;
    let marker = "- ";
    if (parent.nodeName === "OL") {
      const start = Number(parent.getAttribute("start") ?? 1);
      marker = `${start + Array.prototype.indexOf.call(parent.children, node)}. `;
    }
    const body = content.replace(/^\n+/, "").replace(/\n+$/, "\n").replace(/\n/gm, `\n${" ".repeat(marker.length)}`);
    return marker + body + (node.nextSibling && !/\n$/.test(body) ? "\n" : "");
  },
});

/** HTML edited in the preview, back as Markdown. */
export function toMarkdown(html: string): string {
  return turndown.turndown(html).trim();
}

export interface MarkdownStats {
  words: number;
  headings: number;
  links: number;
  codeBlocks: number;
}

export function markdownStats(text: string): MarkdownStats {
  const outside = text.replace(/^ {0,3}(`{3,}|~{3,})[^\n]*\n[\s\S]*?(^ {0,3}\1[^\n]*$|(?![\s\S]))/gm, "");
  return {
    words: outside.match(/[\p{L}\p{N}][\p{L}\p{N}'’-]*/gu)?.length ?? 0,
    headings: outside.match(/^ {0,3}#{1,6}\s/gm)?.length ?? 0,
    links: outside.match(/\]\([^)]*\)/g)?.length ?? 0,
    codeBlocks: (text.match(/^ {0,3}(`{3,}|~{3,})/gm)?.length ?? 0) >> 1,
  };
}
