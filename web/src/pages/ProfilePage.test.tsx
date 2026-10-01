// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within
} from "@testing-library/react";
import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  PropsWithChildren,
  ReactNode,
  SelectHTMLAttributes
} from "react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  applicationBootstrapQueryKey,
  signedInContextQueryKey,
  type ApplicationBootstrap
} from "@/applicationSession";
import {
  disconnectExternalGitConnection,
  getExternalGitConnectionStatus,
  listPersonalAccessTokens,
  updateDisplayName,
  type AuthUser,
  type ExternalGitConnectionStatus,
  type ExternalGitProvider
} from "@/lib/api";
import type { Translator } from "@/lib/i18n";
import { ProfilePage } from "@/pages/ProfilePage";

vi.mock("@/components/ui", () => ({
  UiBadge: ({ children }: PropsWithChildren) => <span>{children}</span>,
  UiButton: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{children}</button>
  ),
  UiCard: ({ children }: PropsWithChildren) => <section>{children}</section>,
  UiDialog: ({
    open,
    children,
    actions
  }: PropsWithChildren<{ open: boolean; actions?: ReactNode }>) =>
    open ? (
      <div role="dialog">
        {children}
        {actions}
      </div>
    ) : null,
  UiEmptyState: ({ description }: { description?: ReactNode }) => <div>{description}</div>,
  UiHelpTooltip: () => null,
  UiIconButton: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{children}</button>
  ),
  UiInput: ({
    label,
    error,
    ...props
  }: InputHTMLAttributes<HTMLInputElement> & { label?: ReactNode; error?: ReactNode }) => (
    <label>
      {label}
      <input {...props} />
      {error ? <span role="alert">{error}</span> : null}
    </label>
  ),
  UiPageHeading: ({ title }: { title: ReactNode }) => <h1>{title}</h1>,
  UiSectionHeading: ({
    title,
    description,
    actions
  }: {
    title: ReactNode;
    description?: ReactNode;
    actions?: ReactNode;
  }) => (
    <div>
      <h2>{title}</h2>
      {description}
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
  UiTooltip: ({ children }: PropsWithChildren) => <>{children}</>
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const original = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...original,
    createPersonalAccessToken: vi.fn(),
    disconnectExternalGitConnection: vi.fn(),
    getExternalGitConnectionStatus: vi.fn(),
    listPersonalAccessTokens: vi.fn(),
    revokePersonalAccessToken: vi.fn(),
    updateDisplayName: vi.fn()
  };
});

const authUser: AuthUser = {
  display_name: "OIDC User",
  email: "ada@example.test",
  session_expires_at: "2026-10-01T12:00:00Z",
  user_id: "user-1",
  username: "ada"
};

const provider: ExternalGitProvider = {
  authorization_path: "/v1/external-git/providers/codeberg/authorize",
  base_url: "https://codeberg.org",
  brand: "codeberg",
  capabilities: {
    repository_creation: true,
    supported_visibilities: ["private", "public"]
  },
  display_name: "Codeberg",
  id: "codeberg",
  kind: "forgejo"
};

function connection(
  restriction: ExternalGitConnectionStatus["disconnect_restriction"]
): ExternalGitConnectionStatus {
  return {
    account_id: "account-1",
    base_url: provider.base_url,
    bound: true,
    can_disconnect: restriction === null,
    configured: true,
    connected: true,
    disconnect_restriction: restriction,
    expires_at: null,
    provider: provider.id,
    provider_name: provider.display_name,
    scopes: [],
    status: "active",
    username: "alice"
  };
}

const t: Translator = (key) => key;

function createQueryClient() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } }
  });
}

function renderProfile(queryClient = createQueryClient()) {
  return render(
    <MemoryRouter initialEntries={["/profile"]}>
      <QueryClientProvider client={queryClient}>
        <ProfilePage
          authUser={authUser}
          externalGitProviders={[provider]}
          locale="en"
          t={t}
        />
      </QueryClientProvider>
    </MemoryRouter>
  );
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("ProfilePage", () => {
  it("does not allow the last provider login method to be disconnected", async () => {
    vi.mocked(getExternalGitConnectionStatus).mockResolvedValue(
      connection("last_login_method")
    );
    vi.mocked(listPersonalAccessTokens).mockResolvedValue({ tokens: [] });

    renderProfile();

    const button = await screen.findByRole("button", {
      name: "profile.externalGitDisconnect"
    });
    expect(button.hasAttribute("disabled")).toBe(true);
    expect(
      screen.getByText(
        "profile.externalGitDisconnectRestriction.last_login_method"
      )
    ).toBeTruthy();
  });

  it("disconnects a provider when another login method remains", async () => {
    vi.mocked(getExternalGitConnectionStatus).mockResolvedValue(connection(null));
    vi.mocked(listPersonalAccessTokens).mockResolvedValue({ tokens: [] });
    vi.mocked(disconnectExternalGitConnection).mockResolvedValue(undefined);

    renderProfile();

    fireEvent.click(
      await screen.findByRole("button", {
        name: "profile.externalGitDisconnect"
      })
    );
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(
      within(dialog).getByRole("button", {
        name: "profile.externalGitDisconnect"
      })
    );
    await waitFor(() => {
      expect(vi.mocked(disconnectExternalGitConnection).mock.calls[0]?.[0]).toBe(
        "codeberg"
      );
    });
    expect(dialog).toBeTruthy();
  });

  it("saves a trimmed display name and refreshes the signed-in account context", async () => {
    vi.mocked(getExternalGitConnectionStatus).mockResolvedValue(connection(null));
    vi.mocked(listPersonalAccessTokens).mockResolvedValue({ tokens: [] });
    vi.mocked(updateDisplayName).mockResolvedValue({
      ...authUser,
      display_name: "Ada Lovelace"
    });
    const queryClient = createQueryClient();
    queryClient.setQueryData<ApplicationBootstrap>(applicationBootstrapQueryKey, {
      authConfig: {} as ApplicationBootstrap["authConfig"],
      experience: {} as ApplicationBootstrap["experience"],
      authUser
    });
    queryClient.setQueryData(signedInContextQueryKey(authUser.user_id), {
      projects: [],
      organizations: [],
      hasAdminAccess: false
    });

    renderProfile(queryClient);
    fireEvent.change(screen.getByLabelText("profile.displayNameLabel"), {
      target: { value: "  Ada Lovelace  " }
    });
    fireEvent.click(screen.getByRole("button", { name: "common.save" }));

    expect(await screen.findByText("profile.displayNameSaved")).toBeTruthy();
    expect(vi.mocked(updateDisplayName).mock.calls[0]?.[0]).toBe("Ada Lovelace");
    expect(
      queryClient.getQueryData<ApplicationBootstrap>(applicationBootstrapQueryKey)
        ?.authUser?.display_name
    ).toBe("Ada Lovelace");
    expect(
      queryClient.getQueryState(signedInContextQueryKey(authUser.user_id))
        ?.isInvalidated
    ).toBe(true);
  });

  it("does not submit an empty or oversized display name", () => {
    vi.mocked(getExternalGitConnectionStatus).mockResolvedValue(connection(null));
    vi.mocked(listPersonalAccessTokens).mockResolvedValue({ tokens: [] });

    renderProfile();
    const input = screen.getByLabelText("profile.displayNameLabel");
    const save = screen.getByRole("button", { name: "common.save" });
    expect(save.hasAttribute("disabled")).toBe(true);

    fireEvent.change(input, { target: { value: "   " } });
    expect(save.hasAttribute("disabled")).toBe(true);

    fireEvent.change(input, { target: { value: "a".repeat(65) } });
    expect(save.hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("alert").textContent).toBe("profile.displayNameInvalid");
    fireEvent.click(save);
    expect(updateDisplayName).not.toHaveBeenCalled();
  });
});
