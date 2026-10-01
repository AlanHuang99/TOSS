import { CodeXml } from "lucide-react";
import "@/components/source-code-link.css";
import type { Translator } from "@/lib/i18n";

/** Links to the source code of the running deployment when an operator configures one. */
export function SourceCodeLink({
  url,
  t,
  className = ""
}: {
  url: string | null | undefined;
  t: Translator;
  className?: string;
}) {
  if (!url) return null;
  return (
    <a
      className={`source-code-link ${className}`.trim()}
      href={url}
      target="_blank"
      rel="noopener noreferrer"
    >
      <CodeXml size={14} aria-hidden />
      <span>{t("common.sourceCode")}</span>
    </a>
  );
}
