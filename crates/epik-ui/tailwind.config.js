/** @type {import('tailwindcss').Config} */
module.exports = {
  // Leptos markup lives in Rust source, so that is where the class names are.
  content: ["./index.html", "./src/**/*.rs"],
  theme: {
    extend: {
      // Every colour resolves to a brand token rather than a literal, so the
      // palette is changed by regenerating tokens.css and nothing else.
      colors: {
        root: "var(--epik-bg-root)",
        surface: "var(--epik-bg-surface)",
        raised: "var(--epik-bg-raised)",
        input: "var(--epik-bg-input)",
        bar: "var(--epik-bg-bar)",
        hover: "var(--epik-bg-hover)",
        active: "var(--epik-bg-active)",
        fg: "var(--epik-text-primary)",
        "fg-secondary": "var(--epik-text-secondary)",
        "fg-muted": "var(--epik-text-muted)",
        "fg-faint": "var(--epik-text-faint)",
        "fg-inverse": "var(--epik-text-inverse)",
        accent: "var(--epik-accent-base)",
        "accent-hover": "var(--epik-accent-hover)",
        "accent-muted": "var(--epik-accent-muted)",
        "on-accent": "var(--epik-accent-on-accent)",
        success: "var(--epik-semantic-success)",
        "success-muted": "var(--epik-semantic-success-muted)",
        warning: "var(--epik-semantic-warning)",
        "warning-muted": "var(--epik-semantic-warning-muted)",
        error: "var(--epik-semantic-error)",
        "error-muted": "var(--epik-semantic-error-muted)",
        line: "var(--epik-border-default)",
        "line-strong": "var(--epik-border-strong)",
      },
      fontFamily: {
        sans: "var(--epik-font-sans)",
        mono: "var(--epik-font-mono)",
      },
    },
  },
  plugins: [],
};
