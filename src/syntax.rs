use crate::AppWindow;
use i_slint_core::{item_tree::ItemRc, items::TextInput, window::WindowInner};
use slint::ComponentHandle;
use std::ops::ControlFlow;

pub fn configure(ui: &AppWindow) {
    configure_filename(ui, ui.get_document_name());
}

pub fn configure_filename(ui: &AppWindow, filename: slint::SharedString) {
    let dark = ui.get_dark_theme();
    let root = ItemRc::new_root(WindowInner::from_pub(ui.window()).component());

    root.visit_descendants(|item| {
        if let Some(input) = i_slint_core::items::ItemRef::downcast_pin::<TextInput>(item.borrow())
        {
            if !input.single_line() {
                input.set_syntax(filename.clone(), dark);
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(())
    });
    ui.window().request_redraw();
}

#[cfg(test)]
mod tests {
    use i_slint_core::syntax_highlight::SyntaxCache;
    #[test]
    fn lua_calls_and_logical_operators_are_colored() {
        let text = "if IsPlayerBot(id) and not IsPlayerInHeroSelectionControl(id) or Check() then\n-- Hidden() and not or\nlocal s = \"Hidden() and not or\"";
        for dark in [false, true] {
            let mut cache = SyntaxCache::default();
            let spans = cache.update(&text.into(), "test.lua".into(), dark);
            for token in [
                "IsPlayerBot",
                "and",
                "not",
                "IsPlayerInHeroSelectionControl",
                "or",
                "Check",
            ] {
                let offset = text.find(token).unwrap();
                assert!(
                    spans[0].iter().any(|span| span.range.contains(&offset)),
                    "{token} dark={dark}"
                );
            }
            for line in [1, 2] {
                let source = text.lines().nth(line).unwrap();
                let offset = source.find("Hidden").unwrap();
                let color = spans[line]
                    .iter()
                    .find(|span| span.range.contains(&offset))
                    .unwrap()
                    .color;
                let operator = source.find("and").unwrap();
                assert_eq!(
                    spans[line]
                        .iter()
                        .find(|span| span.range.contains(&operator))
                        .unwrap()
                        .color,
                    color
                );
            }
        }
    }

    #[test]
    fn new_language_tokens_and_multiline_edits_match_fresh_parse() {
        for (file, text, tokens) in [
            ("plugin.sma", "public plugin_init() { new value = 42; server_print(\"hello\"); }", ["public", "42", "hello"]),
            ("plugin.sp", "public Action OnClientCommand(int client) { PrintToChat(client, \"hello\"); return Plugin_Handled; }", ["public", "int", "hello"]),
            ("player.gd", "@export var speed: float = 42.0; var label = \"hello\"", ["@export", "float", "hello"]),
            ("Example.java", "public class Example { int value = 42; String name = \"hello\"; }", ["public", "42", "hello"]),
            ("Example.cs", "public class Example { int value = 42; string name = \"hello\"; }", ["public", "42", "hello"]),
        ] {
            for dark in [false, true] {
                let mut cache = SyntaxCache::default();
                let spans = cache.update(&text.into(), file.into(), dark);
                for token in tokens {
                    let offset = text.find(token).unwrap();
                    assert!(spans[0].iter().any(|span| span.range.contains(&offset)), "{file}: {token}");
                }
                let (original, changed) = if file.ends_with(".gd") {
                    (format!("\"\"\"start\nbody\n\"\"\"\n{text}"), format!("# start\nbody\n\"\"\"\n{text}"))
                } else {
                    (format!("/* start\nbody\n*/\n{text}"), format!("// start\nbody\n*/\n{text}"))
                };
                cache.update(&original.into(), file.into(), dark);
                let incremental = cache.update(&changed.clone().into(), file.into(), dark);
                let fresh = SyntaxCache::default().update(&changed.into(), file.into(), dark);
                assert_eq!(incremental, fresh, "{file} dark={dark}");
            }
        }
    }

    #[test]
    fn cmake_names_and_bracket_states_are_cached_correctly() {
        for name in [
            "CMakeLists.txt",
            "CMAKELists.TXT",
            "nested/CMakeLists.txt",
            "module.cmake",
            "module.CMAKE",
        ] {
            let mut cache = SyntaxCache::default();
            for original in [
                "#[=[comment\nbody\n]=]\nset(VALUE \"${NAME}\")",
                "[==[string\nbody\n]==]\nmessage(STATUS \"hello\")",
            ] {
                let spans = cache.update(&original.into(), name.into(), true);
                assert!(!spans[1].is_empty(), "{name}");
                let changed = original.replacen("body", "changed", 1);
                let incremental = cache.update(&changed.clone().into(), name.into(), true);
                let fresh = SyntaxCache::default().update(&changed.into(), name.into(), true);
                assert_eq!(incremental, fresh);
                assert!(spans
                    .last()
                    .unwrap()
                    .iter()
                    .any(|span| span.range.start == 0));
            }
        }
    }

    #[test]
    fn common_formats_and_multiline_state_are_highlighted() {
        for (file, text) in [
            ("test.lua", "-- comment\nlocal value = \"string\"\n"),
            ("test.cpp", "// comment\nint main() { return 42; }"),
            ("test.c", "/* comment */\nint value = 3;"),
            ("test.h", "#define VALUE 1\nstruct Thing {};"),
            ("test.py", "# comment\ndef example():\n    return 'text'"),
            ("test.rs", "// comment\nfn main() { let value = 42; }"),
            ("test.json", "{\"key\": true}"),
            ("test.js", "// comment\nconst value = 42;"),
            ("test.html", "<div class=\"name\">text</div>"),
            ("test.css", "body { color: red; }"),
            ("test.yaml", "key: true\nvalue: 42"),
            ("test.toml", "[section]\nvalue = 42"),
            ("test.sh", "#!/bin/sh\necho \"text\""),
            ("test.go", "package main\nfunc main() { return }"),
            ("test.sma", "#include <amxmodx>\npublic plugin_init() { new Float:value = 42.0; server_print(\"hello\"); }"),
            ("test.sp", "#include <sourcemod>\npublic Action OnClientCommand(int client) { PrintToChat(client, \"hello\"); return Plugin_Handled; }"),
            ("test.gd", "extends Node\n@export var value: int = 42\nfunc _ready():\n    print(\"hello\")"),
            ("test.cs", "using System;\npublic class Example { public string Value = \"hello\"; }"),
            ("CMakeLists.txt", "cmake_minimum_required(VERSION 3.20)\nproject(Example)\nset(NAME \"hello\")"),
            ("module.cmake", "# comment\nset(VALUE 42)\nmessage(STATUS \"${VALUE}\")"),
            ("test.csx", "var value = \"hello\"; Console.WriteLine(value);"),
            ("test.java", "public class Example { int value = 42; }"),
            ("test.rb", "# comment\ndef example\n  42\nend"),
            ("test.php", "<?php echo \"text\"; ?>"),
            ("test.sql", "SELECT * FROM example WHERE value = 42;"),
            ("test.ts", "const value: number = 42;"),
            ("test.md", "# Heading\n**strong**"),
        ] {
            let mut cache = SyntaxCache::default();
            let spans = cache.update(&text.into(), file.into(), true);
            assert!(spans.iter().any(|line| !line.is_empty()), "{file}");
        }
        let mut cache = SyntaxCache::default();
        cache.update(
            &"/* start\nstill comment\n*/ int value;".into(),
            "test.c".into(),
            true,
        );
        let changed = cache.update(
            &"// start\nstill comment\n*/ int value;".into(),
            "test.c".into(),
            true,
        );
        let mut fresh = SyntaxCache::default();
        let expected = fresh.update(
            &"// start\nstill comment\n*/ int value;".into(),
            "test.c".into(),
            true,
        );
        for (a, b) in changed.iter().zip(expected.iter()) {
            assert_eq!(
                a.iter()
                    .map(|span| (&span.range, span.color))
                    .collect::<Vec<_>>(),
                b.iter()
                    .map(|span| (&span.range, span.color))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[cfg(test)]
mod cache_tests {
    use i_slint_core::syntax_highlight::SyntaxCache;
    use std::rc::Rc;
    #[test]
    fn unchanged_parser_tail_is_reused_and_plain_text_stays_plain() {
        let mut cache = SyntaxCache::default();
        let before = cache.update(
            &"local a = 1\nprint(a)\nreturn a".into(),
            "test.lua".into(),
            true,
        );
        let changed: slint::SharedString = "local a = 2\nprint(a)\nreturn a".into();
        let after = cache.update(&changed, "test.lua".into(), true);
        assert!(Rc::ptr_eq(&before[1], &after[1]));
        let unchanged = cache.update(&changed, "test.lua".into(), true);
        assert!(Rc::ptr_eq(&after, &unchanged));
        assert!(cache
            .update(&"some plain text".into(), "notes.txt".into(), true)
            .is_empty());
    }
}
