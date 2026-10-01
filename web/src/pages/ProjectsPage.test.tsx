// @vitest-environment jsdom

import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  PropsWithChildren,
  ReactNode,
  SelectHTMLAttributes
} from "react";
import { MemoryRouter } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createProject, deleteProject, type Project } from "@/lib/api";
import type { Translator } from "@/lib/i18n";
import { ProjectsPage } from "@/pages/ProjectsPage";
import { ApplicationRuntimeProvider } from "@/composition/applicationRuntime";
import { coreProjectCatalog } from "@/projects/coreProjectCatalog";
import { createTestApplicationRuntime } from "@/testSupport/applicationRuntime";

vi.mock("@/lib/api", () => ({
  copyProject: vi.fn(),
  deleteProject: vi.fn(),
  getProcessingCapabilities: vi.fn(),
  createProject: vi.fn(),
  listProjects: vi.fn(),
  projectThumbnailUrl: vi.fn(),
  renameProject: vi.fn(),
  setProjectArchived: vi.fn(),
  updateProjectDescription: vi.fn(),
  uploadProjectThumbnail: vi.fn()
}));

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
  UiCard: ({ children }: PropsWithChildren) => <div>{children}</div>,
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
  UiEmptyState: ({ description }: { description?: ReactNode }) => <div>{description}</div>,
  UiIconButton: ({
    children,
    tooltip: _tooltip,
    label,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement> & { tooltip: string; label: string }) => (
    <button {...props} aria-label={label}>{children}</button>
  ),
  UiInput: ({
    label,
    error,
    ...props
  }: InputHTMLAttributes<HTMLInputElement> & { label?: ReactNode; error?: ReactNode }) => (
    <label>
      {label}
      <input {...props} />
      {error && <span role="alert">{error}</span>}
    </label>
  ),
  UiPageHeading: ({ title }: { title: ReactNode }) => <h1>{title}</h1>,
  UiSectionHeading: ({
    title,
    actions
  }: {
    title: ReactNode;
    actions?: ReactNode;
  }) => (
    <div>
      <h2>{title}</h2>
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
  )
}));

vi.mock("@/pages/projects/ExternalGitImportDialog", () => ({
  ExternalGitImportDialog: () => null
}));

const t: Translator = (key) => key;

const existingProject: Project = {
  archived: false,
  archived_at: null,
  can_read: true,
  created_at: "2026-07-13T00:00:00Z",
  description: null,
  has_thumbnail: false,
  id: "project-a",
  is_template: false,
  last_edited_at: "2026-07-13T00:00:00Z",
  latex_engine: null,
  my_role: "Owner",
  name: "quarterly-review",
  owner_display_name: "user-a",
  owner_user_id: "user-a",
  project_type: "typst"
};

function renderPage(
  projects: Project[] = [],
  refreshProjects = vi.fn().mockResolvedValue(undefined)
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } }
  });
  render(
    <QueryClientProvider client={queryClient}>
      <ApplicationRuntimeProvider
        runtime={createTestApplicationRuntime({ projects: coreProjectCatalog })}
      >
        <MemoryRouter>
          <ProjectsPage
            projects={projects}
            organizations={[]}
            enabledProjectTypes={["typst"]}
            externalGitProviders={[]}
            refreshProjects={refreshProjects}
            locale="en"
            t={t}
          />
        </MemoryRouter>
      </ApplicationRuntimeProvider>
    </QueryClientProvider>
  );
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("ProjectsPage", () => {
  it("shows and focuses the project-name error instead of silently ignoring creation", () => {
    renderPage();

    fireEvent.click(screen.getByRole("button", { name: "projects.createAction" }));

    expect(createProject).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toBe("projects.nameRequired");
    expect(document.activeElement).toBe(screen.getByPlaceholderText("projects.namePlaceholder"));
  });

  it("rejects a visible duplicate name without making a creation request", () => {
    renderPage([existingProject]);
    const nameInput = screen.getByPlaceholderText("projects.namePlaceholder");
    fireEvent.change(nameInput, { target: { value: "  QUARTERLY-REVIEW  " } });

    fireEvent.click(screen.getByRole("button", { name: "projects.createAction" }));

    expect(createProject).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toBe("projects.nameDuplicate");
    expect(document.activeElement).toBe(nameInput);
  });

  it("offers deletion only to owners", () => {
    renderPage([
      existingProject,
      {
        ...existingProject,
        id: "project-b",
        my_role: "ReadWrite",
        name: "shared-notes"
      }
    ]);

    expect(screen.getAllByRole("button", { name: "projects.delete" })).toHaveLength(1);
  });

  it("deletes a project only after its name is typed", async () => {
    vi.mocked(deleteProject).mockResolvedValue(undefined);
    const refreshProjects = vi.fn().mockResolvedValue(undefined);
    renderPage([existingProject], refreshProjects);

    fireEvent.click(screen.getByRole("button", { name: "projects.delete" }));
    const dialog = screen.getByRole("dialog", { name: "projects.deleteDialogTitle" });
    const confirm = within(dialog).getByRole("button", { name: "projects.deleteAction" });
    const confirmation = within(dialog).getByLabelText("projects.deleteConfirmLabel");
    expect(confirm.hasAttribute("disabled")).toBe(true);

    fireEvent.change(confirmation, { target: { value: "quarterly" } });
    fireEvent.click(confirm);
    expect(deleteProject).not.toHaveBeenCalled();

    fireEvent.change(confirmation, { target: { value: "quarterly-review" } });
    expect(confirm.hasAttribute("disabled")).toBe(false);
    fireEvent.click(confirm);

    await waitFor(() => expect(deleteProject).toHaveBeenCalledWith("project-a"));
    await waitFor(() => expect(refreshProjects).toHaveBeenCalled());
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
