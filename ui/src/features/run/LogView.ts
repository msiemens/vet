// Draws a RunLog into a scrolling element without Svelte: output is appended to the DOM
// once per frame, however many chunks arrived, and the log never re-renders as a whole.

import type { RunLog } from "../../state/runLog";

/** Text a span takes before the next output starts a new one; trimming works per span. */
const SPAN_CHARS = 16_384;
/** Within this distance of the end the log follows new output. */
const FOLLOW_PX = 4;

interface Span {
  text: Text;
  stream: string;
  size: number;
  /** The newest piece in it. */
  last: number;
}

export class LogView {
  private spans: Span[] = [];
  private next = 0;
  private frame = 0;

  constructor(
    private el: HTMLElement,
    private log: RunLog,
  ) {
    log.onchange = () => {
      this.frame ||= requestAnimationFrame(() => this.flush());
    };
    this.flush();
  }

  destroy() {
    cancelAnimationFrame(this.frame);
    this.log.onchange = undefined;
    this.el.replaceChildren();
    delete this.el.dataset.trimmed;
  }

  private flush() {
    this.frame = 0;
    const { el, log } = this;
    const pieces = log.since(this.next);
    if (!pieces.length) return;
    this.next = log.end;
    const follow = el.scrollHeight - el.scrollTop - el.clientHeight <= FOLLOW_PX;

    const added = document.createDocumentFragment();
    for (const piece of pieces) {
      let span = this.spans.at(-1);
      if (span?.stream === piece.stream && span.size < SPAN_CHARS) {
        span.text.appendData(piece.text);
        span.size += piece.text.length;
        span.last = piece.seq;
      } else {
        const node = document.createElement("span");
        if (piece.stream === "stderr") node.className = "err";
        const text = document.createTextNode(piece.text);
        node.append(text);
        added.append(node);
        span = { text, stream: piece.stream, size: piece.text.length, last: piece.seq };
        this.spans.push(span);
      }
    }
    el.append(added);

    let dropped = 0;
    while (dropped < this.spans.length - 1 && this.spans[dropped].last < log.first) dropped++;
    for (const span of this.spans.splice(0, dropped)) span.text.parentElement?.remove();
    if (log.trimmed) el.dataset.trimmed = "";
    if (follow) el.scrollTop = el.scrollHeight;
  }
}
