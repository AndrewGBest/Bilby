#!/usr/bin/env python3
"""Bilby's translations: gather the English, write what Slint reads.

    python3 scripts/i18n.py          gather, then write the .po files
    python3 scripts/i18n.py --check  the same, but fail if anything would change

The English text is written once, where it is used:
  * `@tr("...")` in crates/app/ui/*.slint (the interface);
  * string literals in the app's and the core's Rust code (notices, errors, what an
    install did), and the summaries in the lists the store carries.

It is gathered into crates/app/i18n/en.json. Each other language is a file beside
it, English text to translated text (a list of forms for text that counts things).
From those, crates/app/i18n/po/<language>/LC_MESSAGES/bilby.po is written for Slint,
which builds the translations into the program.
"""
import glob, json, os, re, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
I18N = os.path.join(ROOT, "crates/app/i18n")
# Code: (its own name, gettext's rule for which form goes with a number, forms).
LANGUAGES = {
    "es": ("Español", "nplurals=2; plural=(n != 1);", 2),
    "fr": ("Français", "nplurals=2; plural=(n > 1);", 2),
    "de": ("Deutsch", "nplurals=2; plural=(n != 1);", 2),
    "pt": ("Português (Brasil)", "nplurals=2; plural=(n > 1);", 2),
    "it": ("Italiano", "nplurals=2; plural=(n != 1);", 2),
    "ru": ("Русский", "nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<12 || n%100>14) ? 1 : 2);", 3),
    "zh": ("简体中文", "nplurals=1; plural=0;", 1),
    "ja": ("日本語", "nplurals=1; plural=0;", 1),
    "ko": ("한국어", "nplurals=1; plural=0;", 1),
    "hi": ("हिन्दी", "nplurals=2; plural=(n != 1);", 2),
    "ar": ("العربية", "nplurals=6; plural=(n==0 ? 0 : n==1 ? 1 : n==2 ? 2 : n%100>=3 && n%100<=10 ? 3 : n%100>=11 ? 4 : 5);", 6),
}
# Messages the code puts together from parts, which no one literal holds.
PUT_TOGETHER = [
    "today", "yesterday", "1 day ago", "{} days ago", "1 month ago", "{} months ago", "1 year ago", "{} years ago",
    "1 rating",
    # The on-screen keyboard's wide keys.
    "space", "delete", "search", "shift", "enter", "done",
]
LITERAL = re.compile(r'"((?:[^"\\]|\\.)*)"')


def unescape(text):
    return (text.replace('\\"', '"').replace("\\n", "\n").replace("\\'", "'").replace("\\\\", "\\"))


def slint_strings():
    """(context, text, plural or None) for every @tr in the interface."""
    found = []
    for path in sorted(glob.glob(os.path.join(ROOT, "crates/app/ui/*.slint"))):
        component = ""
        for line in open(path, encoding="utf-8"):
            declared = re.match(r"\s*(?:export\s+)?(?:component|global)\s+([A-Za-z0-9_-]+)", line)
            if declared and not line.startswith((" ", "\t")):
                component = declared.group(1)
            if line.strip().startswith("//"):
                continue
            for at in re.finditer(r"@tr\(", line):
                rest = line[at.end():]
                first = LITERAL.match(rest)
                if not first:
                    raise SystemExit(f"{path}: @tr without a text: {line.strip()[:80]}")
                context, text, after = component, first.group(1), rest[first.end():]
                arrow = re.match(r"\s*=>\s*", after)
                if arrow:
                    second = LITERAL.match(after[arrow.end():])
                    context, text, after = text, second.group(1), after[arrow.end() + second.end():]
                bar = re.match(r"\s*\|\s*", after)
                plural = LITERAL.match(after[bar.end():]).group(1) if bar else None
                found.append((unescape(context), unescape(text), unescape(plural) if plural else None))
    return found


def gap(text):
    """Rust's "{name}" and "{:?}" are all one kind of gap to a translator: "{}"."""
    text = text.replace("{{", "\x00").replace("}}", "\x01")
    text = re.sub(r"\{[^{}]*\}", "{}", text)
    return text.replace("\x00", "{").replace("\x01", "}")


def rust_strings():
    """Text the Rust code puts on screen: literals of two words or more outside tests."""
    found = []
    paths = sorted(glob.glob(os.path.join(ROOT, "crates/core/src/**/*.rs"), recursive=True))
    paths += [os.path.join(ROOT, "crates/app/src", name) for name in ("lib.rs", "activation.rs")]
    for path in paths:
        if path.endswith(("demo.rs", "i18n.rs")):
            continue
        source = open(path, encoding="utf-8").read()
        tests = source.find("#[cfg(test)]")
        source = source[:tests] if tests > 0 else source
        # A literal may run over several lines with a trailing backslash.
        source = re.sub(r"\\\n\s*", "", source)
        own = path.endswith(("lib.rs", "os.rs", "categories.rs"))
        for line in source.split("\n"):
            stripped = line.strip()
            if stripped.startswith(("//", "#[", "eprintln!", "println!", "debug_assert", "assert")) or ".expect(" in line:
                continue
            # Names of folders and marks in files, which nobody is shown.
            if "(not shown)" in line:
                continue
            for literal in LITERAL.finditer(line):
                text = unescape(literal.group(1))
                # Two words, or a word beside something filled in ("{} ratings").
                words = re.search(r"[A-Za-z]{2,}[ ’'][A-Za-z]{2,}|\{[^{}]*\} ?[A-Za-z]{3,}|[A-Za-z]{3,} ?\{[^{}]*\}", text)
                one_word = own and re.fullmatch(r"[A-Z][a-z]{2,}(…|\.)?", text)
                # Commands, file contents and the like are not for people to read.
                code = "\n" in text or re.search(r"[;=$<>|\\]|--|\./|^[a-z-]+ [a-z-]+ ", text)
                if not (words or one_word) or code or text.startswith(("http", "/", "bilby ")):
                    continue
                found.append(gap(text))
    return found


def data_strings():
    """The summaries and labels in the lists the store carries."""
    found = []

    def walk(node, keys):
        if isinstance(node, dict):
            for key, value in node.items():
                if key in keys and isinstance(value, str) and re.search(r"[A-Za-z]{2,}", value):
                    found.append(value)
                else:
                    walk(value, keys)
        elif isinstance(node, list):
            for value in node:
                walk(value, keys)

    for name, keys in (("essentials", {"summary"}), ("web", {"summary"}), ("index", {"summary", "description"}), ("related", {"label", "note", "summary"})):
        data = json.load(open(os.path.join(ROOT, "crates/core/data", name + ".json"), encoding="utf-8"))
        data.pop("_comment", None) if isinstance(data, dict) else None
        walk(data, keys)
    return found


def gaps_of(text):
    """How many things are filled in. "{}" and "{0}" are the same gap written two ways."""
    names = re.findall(r"\{([^{}]*)\}", text)
    return (len([n for n in names if n != "n"]), "n" in names)


def po_text(text):
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n") + '"'


def po_for(code, translations, interface):
    name, rule, forms = LANGUAGES[code]
    out = ['msgid ""', 'msgstr ""', f'"Language: {code}\\n"', '"Content-Type: text/plain; charset=UTF-8\\n"', f'"Plural-Forms: {rule}\\n"', ""]
    for context, text, plural in interface:
        translated = translations.get(text)
        if plural:
            if not (isinstance(translated, list) and len(translated) == forms):
                continue
            out += [f"msgctxt {po_text(context)}", f"msgid {po_text(text)}", f"msgid_plural {po_text(plural)}"]
            out += [f"msgstr[{n}] {po_text(form)}" for n, form in enumerate(translated)]
        else:
            if not (isinstance(translated, str) and translated):
                continue
            out += [f"msgctxt {po_text(context)}", f"msgid {po_text(text)}", f"msgstr {po_text(translated)}"]
        out.append("")
    return "\n".join(out)


def main():
    check = "--check" in sys.argv
    interface = sorted(set(slint_strings()), key=lambda entry: (entry[1], entry[0]))
    plurals = sorted({(text, plural) for _, text, plural in interface if plural} | {("{n} app", "{n} apps"), ("{n} update", "{n} updates")})
    counted = {text for text, _ in plurals}
    ui = sorted({text for _, text, plural in interface if not plural})
    messages = sorted(set(rust_strings()) | set(data_strings()) | set(PUT_TOGETHER))
    english = {"interface": ui, "counted": [list(pair) for pair in plurals], "messages": [m for m in messages if m not in counted]}
    wanted = {os.path.join(I18N, "en.json"): json.dumps(english, ensure_ascii=False, indent=1) + "\n"}
    every = set(ui) | set(english["messages"]) | counted
    for code in LANGUAGES:
        path = os.path.join(I18N, code + ".json")
        translations = json.load(open(path, encoding="utf-8")) if os.path.exists(path) else {}
        for text, translated in sorted(translations.items()):
            for form in translated if isinstance(translated, list) else [translated]:
                if gaps_of(text) != gaps_of(form):
                    print(f"{code}: the gaps differ: {text!r} -> {form!r}")
            if isinstance(translated, list) != (text in counted) or (text in counted and len(translated) != LANGUAGES[code][2]):
                print(f"{code}: {text!r} needs {LANGUAGES[code][2] if text in counted else 'one'} form(s)")
        missing = sorted(every - set(translations))
        stale = sorted(set(translations) - every)
        if missing or stale:
            print(f"{code}: {len(missing)} not translated, {len(stale)} no longer used")
        if not os.path.exists(path):
            wanted[path] = "{}\n"
        wanted[os.path.join(I18N, "po", code, "LC_MESSAGES", "bilby.po")] = po_for(code, translations, interface)
    changed = []
    for path, text in wanted.items():
        if not os.path.exists(path) or open(path, encoding="utf-8").read() != text:
            changed.append(os.path.relpath(path, ROOT))
            if not check:
                os.makedirs(os.path.dirname(path), exist_ok=True)
                open(path, "w", encoding="utf-8").write(text)
    print(f"{len(ui)} interface texts, {len(plurals)} counted, {len(english['messages'])} messages")
    if check and changed:
        raise SystemExit("out of date, run scripts/i18n.py: " + ", ".join(changed))


if __name__ == "__main__":
    main()
