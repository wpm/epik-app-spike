#!/usr/bin/env python3
"""Turn Epik's brand.json into the CSS custom properties the frontend uses.

`brand.json` here is a verbatim copy of `website/brand/brand.json` in
epik-agent/Epik. Refresh it with:

    gh api repos/epik-agent/Epik/contents/website/brand/brand.json \
      --jq '.content' | base64 -d > crates/epik-ui/brand/brand.json

then run this script to regenerate `styles/tokens.css`.

The palette becomes custom properties rather than Tailwind config entries so
that both themes live in one stylesheet and switching theme is a `data-theme`
attribute on `<html>` rather than a rebuild.
"""

import json
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
BRAND = HERE / "brand.json"
OUT = HERE.parent / "styles" / "tokens.css"

THEMES = (
    ("dark", ':root, :root[data-theme="dark"]'),
    ("light", ':root[data-theme="light"]'),
)


def flatten(prefix, obj, out):
    """`{"bg": {"root": "#0a0a0a"}}` -> `[("-bg-root", "#0a0a0a")]`."""
    for key, value in obj.items():
        # camelCase -> kebab-case, so the variable names read like CSS.
        kebab = "".join("-" + c.lower() if c.isupper() else c for c in key)
        name = f"{prefix}-{kebab}"
        if isinstance(value, dict):
            flatten(name, value, out)
        else:
            out.append((name, value))


def main():
    brand = json.loads(BRAND.read_text())
    lines = [
        "/* Design tokens vendored from Epik's brand.json.",
        " *",
        " * GENERATED — do not edit. Regenerate with:",
        " *   python3 crates/epik-ui/brand/generate-tokens.py",
        " * after updating brand/brand.json from epik-agent/Epik website/brand/brand.json.",
        " *",
        " * Emitted as CSS custom properties rather than baked into Tailwind's config so",
        " * the two themes are one stylesheet and a theme switch is a class on <html>,",
        " * not a rebuild.",
        " */",
        "",
    ]
    for theme, selector in THEMES:
        tokens = []
        flatten("", brand["palette"][theme], tokens)
        lines.append(f"{selector} {{")
        for name, value in tokens:
            lines.append(f"  --epik{name}: {value};")
        if theme == "dark":
            sans = brand["fonts"]["sans"]["family"]
            mono = brand["fonts"]["mono"]["family"]
            lines.append(
                f"  --epik-font-sans: '{sans}', ui-sans-serif, system-ui, sans-serif;"
            )
            lines.append(f"  --epik-font-mono: '{mono}', ui-monospace, monospace;")
        lines.append("}")
        lines.append("")

    OUT.write_text("\n".join(lines))
    print(f"wrote {OUT.relative_to(HERE.parent.parent.parent)}")


if __name__ == "__main__":
    main()
