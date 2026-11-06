import assert from "node:assert/strict";
import test from "node:test";
import { createElement, type ButtonHTMLAttributes } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { JSDOM } from "jsdom";
import { ControlGuide } from "../dist/guides/control-guide.js";
import { GestureGuide } from "../dist/guides/gesture-guide.js";

const Button = ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: string; size?: string }) => createElement("button", props, children);

test("donor control and gesture guides render as illustrations with an injected host button", () => {
  const whiteboard = renderToStaticMarkup(createElement(ControlGuide, { kind: "whiteboard", Button }));
  assert.match(whiteboard, /Example: click Screen, then choose Whiteboard/);
  assert.match(whiteboard, /Replay Screen and Whiteboard guide/);
  const finish = renderToStaticMarkup(createElement(ControlGuide, { kind: "finish", Button }));
  assert.match(finish, /<figcaption[^>]*>Example — Finish and copy<\/figcaption>/);
  const shortcut = renderToStaticMarkup(createElement(GestureGuide, { kind: "record", shortcut: "Ctrl+Shift+R", Button }));
  assert.match(shortcut, /Shortcut key illustration/);
  assert.match(shortcut, /<kbd[^>]*>Ctrl<\/kbd>/);
  assert.match(shortcut, /<kbd[^>]*>R<\/kbd>/);
  const drawing = renderToStaticMarkup(createElement(GestureGuide, { kind: "draw", shortcut: "", Button }));
  assert.match(drawing, /Hold Shift and move your pointer to draw/);
  assert.match(drawing, /native-gesture-ink/);
  assert.doesNotMatch(shortcut + drawing, /data-stage=|setup complete|shortcut applied/i);
});

test("built onboarding styles include donor animation and reduced-motion guidance", async () => {
  const { readFile } = await import("node:fs/promises");
  const css = await readFile(new URL("../dist/styles.css", import.meta.url), "utf8");
  const control = await readFile(new URL("../dist/guides/control-guide.css", import.meta.url), "utf8");
  const gesture = await readFile(new URL("../dist/guides/gesture-guide.css", import.meta.url), "utf8");
  assert.match(css, /guides\/control-guide\.css/);
  assert.match(css, /guides\/gesture-guide\.css/);
  assert.match(control, /prefers-reduced-motion/);
  assert.match(control, /@keyframes control-menu-pointer/);
  assert.match(gesture, /prefers-reduced-motion/);
  assert.match(gesture, /@keyframes guide-drag/);
});

test("replay restarts the illustrated control sequence without dispatching onboarding commands", async () => {
  const dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window });
  Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document });
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { configurable: true, value: true });
  const root = createRoot(dom.window.document.getElementById("root")!);
  await act(async () => { root.render(createElement(ControlGuide, { kind: "whiteboard", Button })); });
  const before = dom.window.document.querySelector(".native-control-motion");
  await act(async () => { dom.window.document.querySelector<HTMLButtonElement>(".native-control-replay")!.click(); });
  const after = dom.window.document.querySelector(".native-control-motion");
  assert.ok(before && after && before !== after, "replay replaces the animated SVG tree to restart its CSS timeline");
  await act(async () => { root.unmount(); });
  dom.window.close();
});
