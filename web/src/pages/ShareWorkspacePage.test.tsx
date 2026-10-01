// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { StrictMode, type PropsWithChildren } from "react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  joinProjectShareLink,
  resolveProjectShareLink,
  type AuthUser
} from "@/lib/api";
import type { Translator } from "@/lib/i18n";
import { ShareWorkspacePage } from "@/pages/ShareWorkspacePage";

vi.mock("@/lib/api", () => ({
  joinProjectShareLink: vi.fn(),
  resolveProjectShareLink: vi.fn()
}));

vi.mock("@/pages/SignInPage", () => ({
  SignInPage: ({ returnTo }: { returnTo?: string }) => (
    <div data-testid="sign-in" data-return-to={returnTo} />
  )
}));

vi.mock("@/pages/WorkspacePage", () => ({
  WorkspacePage: ({ shareSaveStatus }: { shareSaveStatus?: string }) => (
    <div>{`workspace:${shareSaveStatus ?? "none"}`}</div>
  )
}));

vi.mock("@/components/ui", () => ({
  UiButton: ({ children }: PropsWithChildren) => <button>{children}</button>,
  UiCard: ({ children }: PropsWithChildren) => <div>{children}</div>
}));

const authUser: AuthUser = {
  display_name: "user-a",
  email: "user-a@example.test",
  session_expires_at: "2026-07-13T00:00:00Z",
  user_id: "user-a",
  username: "user-a"
};

const t: Translator = (key) => key;

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } }
  });
  return function AppProviders({ children }: PropsWithChildren) {
    return (
      <StrictMode>
        <QueryClientProvider client={client}>
          <MemoryRouter initialEntries={["/share/token-a"]}>
            <Routes>
              <Route path="/share/:token" element={children} />
            </Routes>
          </MemoryRouter>
        </QueryClientProvider>
      </StrictMode>
    );
  };
}

afterEach(() => vi.clearAllMocks());

describe("ShareWorkspacePage", () => {
  it("resolves and joins a signed-in share only once in StrictMode", async () => {
    vi.mocked(resolveProjectShareLink).mockResolvedValue({
      anonymous_mode: "off",
      is_template: false,
      permission: "write",
      project_id: "project-a",
      project_name: "project-a"
    });
    vi.mocked(joinProjectShareLink).mockResolvedValue({
      project_id: "project-a",
      role: "ReadWrite"
    });
    const refreshProjects = vi.fn().mockResolvedValue(undefined);

    render(
      <ShareWorkspacePage
        authUser={authUser}
        authConfig={null}
        projects={[]}
        organizations={[]}
        refreshProjects={refreshProjects}
        locale="en"
        t={t}
        onLocaleChange={vi.fn()}
        onSignedIn={vi.fn().mockResolvedValue(undefined)}
        onLogoutFromWorkspace={vi.fn().mockResolvedValue(undefined)}
      />,
      { wrapper: wrapper() }
    );

    await screen.findByText("workspace:saved");
    await waitFor(() => {
      expect(resolveProjectShareLink).toHaveBeenCalledOnce();
      expect(joinProjectShareLink).toHaveBeenCalledOnce();
      expect(refreshProjects).toHaveBeenCalledOnce();
    });
  });

  it("returns a signed-out visitor to the invitation after signing in", async () => {
    vi.mocked(resolveProjectShareLink).mockResolvedValue({
      anonymous_mode: "off",
      is_template: false,
      permission: "write",
      project_id: "project-a",
      project_name: "project-a"
    });

    render(
      <ShareWorkspacePage
        authUser={null}
        authConfig={null}
        projects={[]}
        organizations={[]}
        refreshProjects={vi.fn().mockResolvedValue(undefined)}
        locale="en"
        t={t}
        onLocaleChange={vi.fn()}
        onSignedIn={vi.fn().mockResolvedValue(undefined)}
        onLogoutFromWorkspace={vi.fn().mockResolvedValue(undefined)}
      />,
      { wrapper: wrapper() }
    );

    const signIn = await screen.findByTestId("sign-in");
    expect(signIn.getAttribute("data-return-to")).toBe("/share/token-a");
    expect(joinProjectShareLink).not.toHaveBeenCalled();
  });
});
