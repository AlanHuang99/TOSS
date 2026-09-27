// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";

const { getDocument } = vi.hoisted(() => ({
  getDocument: vi.fn(),
}));

vi.mock("pdfjs-dist", () => ({
  GlobalWorkerOptions: { workerSrc: "" },
  getDocument,
}));

import { renderPdfBytesToCanvas } from "@/lib/pdf";

describe("PDF rendering", () => {
  beforeEach(() => {
    getDocument.mockReset();
    getDocument.mockReturnValue({
      promise: Promise.resolve({ numPages: 0 }),
      destroy: vi.fn().mockResolvedValue(undefined),
    });
  });

  it("configures packed PDF.js CMaps using the application base URL", async () => {
    await renderPdfBytesToCanvas(document.createElement("div"), new Uint8Array([1, 2, 3]));

    expect(getDocument).toHaveBeenCalledWith({
      data: new Uint8Array([1, 2, 3]).buffer,
      cMapUrl: "/pdfjs/cmaps/",
      cMapPacked: true,
    });
  });
});
