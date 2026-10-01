// @vitest-environment jsdom

import { render, screen } from "@testing-library/react";
import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  PropsWithChildren,
  ReactNode,
  SelectHTMLAttributes
} from "react";
import { describe, expect, it, vi } from "vitest";
import type { AuthConfig, Experience } from "@/lib/api";
import type { Translator } from "@/lib/i18n";
import { HomePage } from "@/pages/HomePage";
import { SignInPage } from "@/pages/SignInPage";

vi.mock("@/components/ui", () => ({
  UiButton: ({
    children,
    variant: _variant,
    size: _size,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: string; size?: string }) => (
    <button {...props}>{children}</button>
  ),
  UiCard: ({ children }: PropsWithChildren) => <section>{children}</section>,
  UiInput: ({ label, ...props }: InputHTMLAttributes<HTMLInputElement> & { label?: ReactNode }) => (
    <label>
      {label}
      <input {...props} />
    </label>
  ),
  UiSelect: ({
    children,
    ...props
  }: SelectHTMLAttributes<HTMLSelectElement> & { label?: ReactNode }) => (
    <select {...props}>{children}</select>
  )
}));

const t: Translator = (key) => key;
const sourceCodeUrl = "https://git.example.test/toss/tree/release";

const experience: Experience = {
  distribution_id: "community",
  landing: {
    headline: { en: "headline", "zh-CN": "headline" },
    highlights: [],
    summary: { en: "summary", "zh-CN": "summary" }
  },
  product: {
    accent_color: "#2563eb",
    accent_text_color: "#ffffff",
    brand_mark: "T",
    description: { en: "description", "zh-CN": "description" },
    name: "Typst Collaboration"
  },
  resources: []
};

const authConfig: AuthConfig = {
  accent_color: "#2563eb",
  accent_text_color: "#ffffff",
  ai_assistant: null,
  allow_local_login: true,
  allow_local_registration: false,
  allow_oidc: false,
  announcement: "",
  anonymous_mode: "off",
  brand_mark: "T",
  client_id: null,
  distribution_id: "community",
  enabled_frontend_features: [],
  enabled_project_types: ["typst"],
  external_git_providers: [],
  groups_claim: "groups",
  identity_providers: [],
  issuer: null,
  redirect_uri: null,
  site_name: "Typst Collaboration",
  site_name_managed: true,
  source_code_url: null
};

function sourceCodeLink() {
  return screen.queryByRole("link", { name: "common.sourceCode" });
}

describe("source code links", () => {
  it("appears on the landing page only when configured", () => {
    const { unmount } = render(
      <HomePage
        experience={experience}
        locale="en"
        t={t}
        onSignIn={vi.fn()}
        onOpenHelp={vi.fn()}
      />
    );
    expect(sourceCodeLink()).toBeNull();
    unmount();

    render(
      <HomePage
        experience={experience}
        locale="en"
        t={t}
        sourceCodeUrl={sourceCodeUrl}
        onSignIn={vi.fn()}
        onOpenHelp={vi.fn()}
      />
    );
    expect(sourceCodeLink()?.getAttribute("href")).toBe(sourceCodeUrl);
  });

  it("appears on the sign-in page only when configured", () => {
    const { unmount } = render(
      <SignInPage
        config={authConfig}
        locale="en"
        t={t}
        onLocaleChange={vi.fn()}
        onSignedIn={vi.fn()}
      />
    );
    expect(sourceCodeLink()).toBeNull();
    unmount();

    render(
      <SignInPage
        config={{ ...authConfig, source_code_url: sourceCodeUrl }}
        locale="en"
        t={t}
        onLocaleChange={vi.fn()}
        onSignedIn={vi.fn()}
      />
    );
    const link = sourceCodeLink();
    expect(link?.getAttribute("href")).toBe(sourceCodeUrl);
    expect(link?.getAttribute("rel")).toBe("noopener noreferrer");
  });
});
