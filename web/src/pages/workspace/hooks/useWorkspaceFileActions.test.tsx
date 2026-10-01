// @vitest-environment jsdom

import { act, renderHook } from "@testing-library/react";
import type { DragEvent, PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  createProjectFile,
  deleteProjectFile,
  downloadProjectArchive,
  moveProjectFile,
  upsertDocumentByPath,
  uploadProjectAsset,
  type Document,
  type ProjectAsset,
} from "@/lib/api";
import { useWorkspaceFileActions } from "@/pages/workspace/hooks/useWorkspaceFileActions";
import { coreWorkspaceBackend } from "@/workspace/coreWorkspaceBackend";
import { ApplicationRuntimeProvider } from "@/composition/applicationRuntime";
import { createTestApplicationRuntime } from "@/testSupport/applicationRuntime";

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    createProjectFile: vi.fn(),
    deleteProjectFile: vi.fn(),
    downloadProjectArchive: vi.fn(),
    moveProjectFile: vi.fn(),
    upsertDocumentByPath: vi.fn(),
    uploadProjectAsset: vi.fn(),
  };
});

function wrapper({ children }: PropsWithChildren) {
  return (
    <ApplicationRuntimeProvider
      runtime={createTestApplicationRuntime({ workspace: coreWorkspaceBackend })}
    >
      {children}
    </ApplicationRuntimeProvider>
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

describe("useWorkspaceFileActions", () => {
  beforeEach(() => {
    for (const mock of [
      createProjectFile,
      deleteProjectFile,
      downloadProjectArchive,
      moveProjectFile,
      upsertDocumentByPath,
      uploadProjectAsset,
    ]) {
      vi.mocked(mock).mockReset();
    }
  });

  it("does not publish an old project mutation after navigation", async () => {
    const creation = deferred<void>();
    vi.mocked(createProjectFile).mockReturnValue(creation.promise);
    const refreshProjectData = vi.fn().mockResolvedValue(undefined);
    const selectActivePath = vi.fn();
    const { result, rerender } = renderHook(
      ({ projectId }: { projectId: string }) =>
        useWorkspaceFileActions({
          projectId,
          sessionGeneration: projectId,
          projectName: projectId,
          projectType: "typst",
          contentEpoch: 0,
          activePath: "main.typ",
          entryFilePath: "main.typ",
          canWrite: true,
          isRevisionMode: false,
          selectActivePath,
          updateDocumentContent: vi.fn(),
          refreshProjectData,
          t: (key) => key,
        }),
      { initialProps: { projectId: "project-a" }, wrapper },
    );
    act(() => {
      result.current.setPathDialog({
        mode: "create",
        kind: "file",
        parentPath: "",
        value: "late.typ",
      });
    });
    let mutation: Promise<void>;
    act(() => {
      mutation = result.current.submitPathDialog();
    });
    rerender({ projectId: "project-b" });

    await act(async () => {
      creation.resolve();
      await mutation;
    });

    expect(refreshProjectData).not.toHaveBeenCalled();
    expect(selectActivePath).not.toHaveBeenCalled();
    expect(result.current.error).toBeNull();
  });

  it("routes uploads by path instead of MIME type", async () => {
    vi.mocked(upsertDocumentByPath).mockResolvedValue({} as Document);
    vi.mocked(uploadProjectAsset).mockResolvedValue({} as ProjectAsset);
    const updateDocumentContent = vi.fn();
    const { result } = renderHook(
      () =>
        useWorkspaceFileActions({
          projectId: "project-a",
          sessionGeneration: "project-a",
          projectName: "project-a",
          projectType: "typst",
          contentEpoch: 3,
          activePath: "main.typ",
          entryFilePath: "main.typ",
          canWrite: true,
          isRevisionMode: false,
          selectActivePath: vi.fn(),
          updateDocumentContent,
          refreshProjectData: vi.fn().mockResolvedValue(undefined),
          t: (key) => key,
        }),
      { wrapper },
    );
    const files = [
      new File(["<style/>"], "apa.csl", { type: "" }),
      new File(["plain"], "notes.dat", { type: "text/plain" }),
    ];

    await act(async () => {
      await result.current.onTreeDrop({
        preventDefault: vi.fn(),
        dataTransfer: { items: [], files },
      } as unknown as DragEvent<HTMLDivElement>);
    });

    expect(vi.mocked(upsertDocumentByPath).mock.calls).toEqual([
      ["project-a", "apa.csl", "<style/>", 3],
    ]);
    expect(updateDocumentContent).toHaveBeenCalledWith("apa.csl", "<style/>");
    expect(vi.mocked(uploadProjectAsset).mock.calls).toHaveLength(1);
    expect(vi.mocked(uploadProjectAsset).mock.calls[0]?.[1]).toMatchObject({
      path: "notes.dat",
      content_type: "text/plain",
    });
    expect(result.current.error).toBeNull();
  });
});
