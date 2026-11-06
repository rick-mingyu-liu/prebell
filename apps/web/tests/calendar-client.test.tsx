// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BoardStart from "../src/components/BoardStart";
import ShareButton from "../src/components/ShareButton";

// The calendar's two client components, run in a DOM: what the share button does with and without a share
// sheet or a clipboard, and when the phone's board is moved to today.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  document.body.replaceChildren();
  history.replaceState(null, "", "/");
  for (const key of ["share", "clipboard"]) Reflect.deleteProperty(navigator, key);
});

const setNavigator = (key: "share" | "clipboard", value: unknown) =>
  Object.defineProperty(navigator, key, { value, configurable: true, writable: true });

describe("ShareButton", () => {
  const render = () =>
    act(() =>
      root.render(
        <ShareButton
          path="/en/earnings?week=2026-11-16"
          title="Market calendar · Week of Nov 16, 2026"
          label="Share"
          copied="Link copied"
          failed="Copy this link:"
        />,
      ),
    );
  const button = () => host.querySelector("button")!;
  const status = () => host.querySelector('[role="status"]')!.textContent;
  const click = () => act(async () => button().click());
  const url = `${location.origin}/en/earnings?week=2026-11-16`;

  it("shows the button once the script runs", () => {
    render();
    expect(button().hidden).toBe(false);
    expect(button().textContent).toBe("Share");
    expect(status()).toBe("");
  });

  it("opens the share sheet where there is one, with the page's absolute link and title", async () => {
    const share = vi.fn().mockResolvedValue(undefined);
    const writeText = vi.fn();
    setNavigator("share", share);
    setNavigator("clipboard", { writeText });
    render();
    await click();
    expect(share).toHaveBeenCalledWith({ title: "Market calendar · Week of Nov 16, 2026", url });
    expect(writeText).not.toHaveBeenCalled();
    expect(status()).toBe("");
  });

  it("stays quiet when the reader closes the share sheet", async () => {
    const writeText = vi.fn();
    setNavigator("share", vi.fn().mockRejectedValue(new DOMException("closed", "AbortError")));
    setNavigator("clipboard", { writeText });
    render();
    await click();
    expect(writeText).not.toHaveBeenCalled();
    expect(status()).toBe("");
  });

  it("copies the link when there is no share sheet, or when the sheet fails", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    setNavigator("clipboard", { writeText });
    render();
    await click();
    expect(writeText).toHaveBeenCalledWith(url);
    expect(status()).toBe("Link copied");

    setNavigator("share", vi.fn().mockRejectedValue(new DOMException("denied", "NotAllowedError")));
    await click();
    expect(writeText).toHaveBeenCalledTimes(2);
    expect(status()).toBe("Link copied");
  });

  it("writes the link out to copy by hand when the clipboard refuses or is missing, without throwing", async () => {
    setNavigator("clipboard", { writeText: vi.fn().mockRejectedValue(new Error("denied")) });
    render();
    await click();
    expect(status()).toBe(`Copy this link: ${url}`);
    expect(host.querySelector(".cal-share-url")!.textContent).toBe(url);
    // An insecure page has no clipboard at all.
    Reflect.deleteProperty(navigator, "clipboard");
    await click();
    expect(status()).toBe(`Copy this link: ${url}`);
  });

  it("clears the last result before trying again, so a repeat is announced afresh", async () => {
    let resolve: () => void = () => {};
    const writeText = vi.fn().mockResolvedValueOnce(undefined).mockImplementationOnce(() => new Promise<void>((r) => (resolve = r)));
    setNavigator("clipboard", { writeText });
    render();
    await click();
    expect(status()).toBe("Link copied");
    await click();
    expect(status()).toBe("");
    await act(async () => resolve());
    expect(status()).toBe("Link copied");
  });
});

describe("BoardStart", () => {
  /** A board laid out as on a phone: a scroller wider inside than out, today's column far to the right. */
  function board({ scrollWidth = 1732, clientWidth = 390, scrollLeft = 0 } = {}) {
    const el = document.createElement("div");
    const col = document.createElement("section");
    col.id = "d-2026-11-06";
    el.append(col);
    document.body.append(el);
    Object.defineProperties(el, {
      scrollWidth: { value: scrollWidth },
      clientWidth: { value: clientWidth },
      scrollLeft: { value: scrollLeft, writable: true },
    });
    Object.defineProperty(col, "offsetLeft", { value: 1392 });
    const scrollTo = vi.fn();
    el.scrollTo = scrollTo as typeof el.scrollTo;
    return scrollTo;
  }
  const mount = () => act(() => root.render(<BoardStart target="d-2026-11-06" />));

  it("moves the phone's board to today's column once, instantly", () => {
    const scrollTo = board();
    mount();
    expect(scrollTo).toHaveBeenCalledTimes(1);
    expect(scrollTo).toHaveBeenCalledWith({ left: 1392, behavior: "instant" });
    // A later render (the page updating) does not pull the board back.
    act(() => root.render(<BoardStart target="d-2026-11-06" />));
    expect(scrollTo).toHaveBeenCalledTimes(1);
  });

  it("never animates, whatever the motion preference", () => {
    window.matchMedia = vi.fn().mockReturnValue({ matches: true, addEventListener() {}, removeEventListener() {} }) as never;
    const scrollTo = board();
    mount();
    expect(scrollTo.mock.calls[0][0].behavior).toBe("instant");
  });

  it("leaves a board the reader has already moved", () => {
    const scrollTo = board({ scrollLeft: 344 });
    mount();
    expect(scrollTo).not.toHaveBeenCalled();
  });

  it("leaves the board alone when it is not a scroller (a desktop) or a link chose the day", () => {
    const desktop = board({ scrollWidth: 1408, clientWidth: 1408 });
    mount();
    expect(desktop).not.toHaveBeenCalled();
    act(() => root.unmount());
    document.body.replaceChildren(host);
    root = createRoot(host);
    history.replaceState(null, "", "/en/earnings#d-2026-11-04");
    const linked = board();
    mount();
    expect(linked).not.toHaveBeenCalled();
  });
});
