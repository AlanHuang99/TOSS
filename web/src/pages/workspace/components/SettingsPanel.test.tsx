// @vitest-environment jsdom

import { fireEvent, render, screen, within } from "@testing-library/react";
import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  PropsWithChildren,
  ReactNode,
  SelectHTMLAttributes,
  TextareaHTMLAttributes
} from "react";
import { describe, expect, it, vi } from "vitest";
import type { Translator } from "@/lib/i18n";
import { SettingsPanel } from "@/pages/workspace/components/SettingsPanel";

vi.mock("@/components/ui", () => ({
  UiBadge: ({ children }: PropsWithChildren) => <span>{children}</span>,
  UiButton: ({
    children,
    variant: _variant,
    size: _size,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: string; size?: string }) => (
    <button {...props}>{children}</button>
  ),
  UiCard: ({ children }: PropsWithChildren) => <section>{children}</section>,
  UiDialog: ({
    open,
    title,
    children,
    actions
  }: PropsWithChildren<{ open: boolean; title?: string; actions?: ReactNode }>) =>
    open ? (
      <div role="dialog" aria-label={title}>
        {children}
        {actions}
      </div>
    ) : null,
  UiHelpTooltip: () => null,
  UiIconButton: ({
    children,
    tooltip: _tooltip,
    label,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement> & { tooltip: string; label: string }) => (
    <button {...props} aria-label={label}>
      {children}
    </button>
  ),
  UiInput: ({
    label,
    error: _error,
    ...props
  }: InputHTMLAttributes<HTMLInputElement> & { label?: ReactNode; error?: ReactNode }) => (
    <label>
      {label}
      <input {...props} />
    </label>
  ),
  UiSectionHeading: ({ title, actions }: { title: ReactNode; actions?: ReactNode }) => (
    <div>
      <h3>{title}</h3>
      {actions}
    </div>
  ),
  UiSelect: ({
    label,
    children,
    ...props
  }: SelectHTMLAttributes<HTMLSelectElement> & { label?: ReactNode }) => (
    <label>
      {label}
      <select {...props}>{children}</select>
    </label>
  ),
  UiTextarea: ({
    label,
    error,
    ...props
  }: TextareaHTMLAttributes<HTMLTextAreaElement> & { label?: ReactNode; error?: ReactNode }) => (
    <label>
      {label}
      <textarea {...props} />
      {error ? <span role="alert">{error}</span> : null}
    </label>
  ),
  UiTooltip: ({ children }: PropsWithChildren) => <>{children}</>
}));

vi.mock("@/pages/workspace/components/ExternalGitSettingsCard", () => ({
  ExternalGitSettingsCard: () => null
}));

const t: Translator = (key) => key;

function renderPanel(overrides: Partial<Parameters<typeof SettingsPanel>[0]> = {}) {
  const props: Parameters<typeof SettingsPanel>[0] = {
    width: 360,
    projectId: "project-a",
    projectName: "Lecture notes",
    projectDescription: "Week one",
    projectType: "typst",
    typstPreviewRenderer: "pdf",
    latexEngine: "xetex",
    entryFilePath: "main.typ",
    typEntryOptions: ["main.typ"],
    canManageProject: true,
    canViewWriteShareLink: true,
    projectAccessEnabled: false,
    externalRepositoriesEnabled: false,
    externalGitProviders: [],
    gitRepoUrl: "",
    copiedControl: null,
    templateEnabled: false,
    myOrganizations: [],
    projectOrgAccess: [],
    projectAccessUsers: [],
    error: null,
    entryFilePending: false,
    latexEnginePending: false,
    descriptionPending: false,
    deletePending: false,
    deleteError: null,
    onEntryFileChange: vi.fn(),
    onLatexEngineChange: vi.fn(),
    onSaveDescription: vi.fn().mockResolvedValue(undefined),
    onDeleteProject: vi.fn().mockResolvedValue(undefined),
    onTypstPreviewRendererChange: vi.fn(),
    onCopyToClipboard: vi.fn(),
    onToggleTemplate: vi.fn(),
    activeReadShare: null,
    activeWriteShare: null,
    onCreateShare: vi.fn(),
    onRevokeShare: vi.fn(),
    onGrantOrgAccess: vi.fn(),
    onRevokeOrgAccess: vi.fn(),
    formatAccessType: () => "",
    formatRoleLabel: () => "",
    formatAccessSource: () => "",
    preferredSection: "project",
    t,
    ...overrides
  };
  render(<SettingsPanel {...props} />);
  return props;
}

describe("SettingsPanel project details", () => {
  it("saves a trimmed description and clears an empty one", () => {
    const props = renderPanel();
    const description = screen.getByLabelText("settings.description");
    const save = screen.getByRole("button", { name: "common.save" });
    expect(save.hasAttribute("disabled")).toBe(true);

    fireEvent.change(description, { target: { value: "  Week two  " } });
    fireEvent.click(save);
    expect(props.onSaveDescription).toHaveBeenLastCalledWith("Week two");

    fireEvent.change(description, { target: { value: "   " } });
    fireEvent.click(save);
    expect(props.onSaveDescription).toHaveBeenLastCalledWith(null);

    fireEvent.change(description, { target: { value: "x".repeat(2001) } });
    expect(screen.getByRole("alert").textContent).toBe("settings.descriptionTooLong");
    expect(save.hasAttribute("disabled")).toBe(true);
  });

  it("deletes the project after the owner confirms its name", () => {
    const props = renderPanel();

    fireEvent.click(screen.getByRole("button", { name: "projects.delete" }));
    const dialog = screen.getByRole("dialog", { name: "projects.deleteDialogTitle" });
    const confirm = within(dialog).getByRole("button", { name: "projects.deleteAction" });
    expect(confirm.hasAttribute("disabled")).toBe(true);

    fireEvent.change(within(dialog).getByLabelText("projects.deleteConfirmLabel"), {
      target: { value: "Lecture notes" }
    });
    fireEvent.click(confirm);
    expect(props.onDeleteProject).toHaveBeenCalledTimes(1);
  });

  it("shows the description read-only and hides deletion for non-owners", () => {
    renderPanel({ canManageProject: false });

    expect(screen.getByLabelText("settings.description").hasAttribute("disabled")).toBe(true);
    expect(screen.queryByRole("button", { name: "common.save" })).toBeNull();
    expect(screen.queryByRole("button", { name: "projects.delete" })).toBeNull();
  });
});
