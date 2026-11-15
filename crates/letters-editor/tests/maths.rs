//! Formulas: `$x$` and `$$x$$` as they are typed, and a caret in their TeX source.

mod common;

use common::Do::{self, K, T};
use common::{Harness, canon, clip};
use letters_model::Mark;

fn play(before: &str, script: &[Do]) -> String {
    let mut editor = Harness::new(before);
    editor.play(script);
    editor.state()
}

/// The TeX the caret is in, when it is in a formula.
fn source(editor: &Harness) -> Option<String> {
    let (block, range) = editor.editor.formula()?;
    Some(editor.editor.doc().block(block)?.plain_text()[range].to_owned())
}

#[test]
fn a_closing_dollar_makes_inline_maths() {
    let cases = [
        ("p |", "$x+2$", "p $x+2$|"),
        ("p so |", "$a+b$ holds", "p so $a+b$ holds|"),
        ("h1 |", "$1+1=2$", "h1 $1+1=2$|"),
        ("p |", "$\\frac{a}{b}$", "p $\\\\frac{a}{b}$|"),
        // Beside other text a doubled pair is the same thing.
        ("p so |", "$$x$$", "p so $x$|"),
        // TeX is kept as typed, whatever shorthand it looks like.
        ("p |", "$f*g*h$", "p $f\\*g\\*h$|"),
        ("p |", "$a_{`i`}[j](k)$", "p $a\\_{\\`i\\`}\\[j\\](k)$|"),
        ("p |", "$\\$5$", "p $\\\\\\$5$|"),
        // Shorthand works again once the formula is closed.
        ("p |", "$x$ and *y*", "p $x$ and *y*|"),
    ];
    for (before, typed, after) in cases {
        assert_eq!(play(before, &[T(typed)]), canon(after), "typing {typed:?} into {before:?}");
    }
    assert_eq!(play("p |", &[T("$x$"), K("Backspace")]), canon("p \\$x\\$|"), "Backspace gives the dollars back");
}

#[test]
fn money_and_stray_dollars_stay_as_typed() {
    let cases = [
        ("p |", "$5 and $6", "p \\$5 and \\$6|"),
        ("p |", "$5$", "p \\$5\\$|"),
        ("p |", "$1,000.50$", "p \\$1,000.50\\$|"),
        ("p |", "$5-$10", "p \\$5-\\$10|"),
        ("p |", "$ x$", "p \\$ x\\$|"),
        ("p |", "$x $", "p \\$x \\$|"),
        ("p |", "a$", "p a\\$|"),
        ("p |", "\\$x$", "p \\\\\\$x\\$|"),
        ("p |", "$$$x$$$", "p \\$\\$\\$x\\$\\$\\$|"),
        // Money does not stop the shorthand after it.
        ("p |", "$5 for *this*", "p \\$5 for *this*|"),
        // Code is as written.
        ("p `a|`", "$x$", "p `a\\$x\\$|`"),
        ("code |", "$x$", "code \\$x\\$|"),
    ];
    for (before, typed, after) in cases {
        assert_eq!(play(before, &[T(typed)]), canon(after), "typing {typed:?} into {before:?}");
    }
}

#[test]
fn a_doubled_dollar_makes_a_formula_block() {
    let cases = [
        // Opened like a code fence, and closed like one.
        ("p |", vec![T("$$"), K("Enter")], "formula |"),
        ("p |", vec![T("$$"), K("Enter"), T("a"), K("Enter"), T("b")], "formula a⏎b|"),
        ("formula a⏎|", vec![T("$$"), K("Enter")], "formula a\np |"),
        ("p |", vec![T("$$"), K("Enter"), K("Backspace")], "p \\$\\$|"),
        // Written on one line, it is done, and the writing goes on below.
        ("p |", vec![T("$$x+2$$")], "formula x+2\np |"),
        ("p |\np next", vec![T("$$a*b*c$$ and")], "formula a\\*b\\*c\np  and|\np next"),
        ("> p |", vec![T("$$x$$")], "> formula x\n> p |"),
        ("p |", vec![T("$$x+2$$"), K("Backspace")], "p \\$\\$x+2\\$\\$|"),
        // In a formula nothing is shorthand.
        ("formula |", vec![T("**a** $b$ # ")], "formula \\*\\*a\\*\\* \\$b\\$ \\# |"),
    ];
    for (before, script, after) in cases {
        assert_eq!(play(before, &script), canon(after), "{script:?} from {before:?}");
    }
    assert!(!Harness::new("formula a|").key("Mod+b"), "a formula block takes no marks");
}

#[test]
fn a_formula_block_is_left_as_code_is() {
    let cases = [
        ("formula a|", "Down", "formula a\np |"),
        ("formula a|", "Right", "formula a\np |"),
        ("formula |a", "Up", "p |\nformula a"),
        ("formula a|b", "Escape", "formula ab\np |"),
        // With text below, the caret goes to it.
        ("formula a|b\np next", "Escape", "formula ab\np |next"),
        ("formula a|\np next", "Down", "formula a\np n|ext"),
    ];
    for (before, key, after) in cases {
        assert_eq!(play(before, &[K(key)]), canon(after), "{key} from {before:?}");
    }
}

#[test]
fn the_caret_goes_into_inline_maths_and_out_again() {
    // Each Right from before the formula: the offset in the block, and the source the caret is in.
    let mut editor = Harness::new("p a |$xy$ b");
    let mut stops = vec![(2, source(&editor))];
    for _ in 0..5 {
        editor.key("Right");
        let offset = editor.editor.selection().expect("the paragraph holds the caret").head.offset;
        stops.push((offset, source(&editor)));
    }
    let inside = || Some("xy".to_owned());
    assert_eq!(stops, [(2, None), (2, inside()), (3, inside()), (4, inside()), (4, None), (5, None)]);

    // The dollars show while the caret touches the formula, as any pair of markers does.
    assert_eq!(Harness::new("p a $x|y$ b").revealed(), "a $x|y$ b");
    assert_eq!(Harness::new("p a $xy$ |b").revealed(), "a xy |b");
}

#[test]
fn typing_inside_a_formula_is_its_source_and_outside_is_not() {
    let cases = [
        ("p $x|y$", vec![T("+")], "p $x+|y$"),
        ("p $x|$", vec![T("y")], "p $xy|$"),
        ("p $x|$", vec![K("Right"), T("y")], "p $x$y|"),
        ("p |$x$", vec![T("y")], "p y|$x$"),
        ("p |$x$", vec![K("Right"), T("y")], "p $y|x$"),
        // Shorthand typed into the source stays TeX.
        ("p $x|$", vec![T("*a*")], "p $x\\*a\\*|$"),
        // Deleting either dollar leaves the source as text.
        ("p $x|$", vec![K("Right"), K("Backspace")], "p x|"),
        ("p |$x$", vec![K("Delete")], "p |x"),
    ];
    for (before, script, after) in cases {
        assert_eq!(play(before, &script), canon(after), "{script:?} from {before:?}");
    }
}

#[test]
fn escape_and_enter_leave_inline_maths_where_it_stands() {
    for key in ["Escape", "Enter"] {
        let mut editor = Harness::new("p a $x|y$ b");
        assert_eq!(source(&editor), Some("xy".into()));
        assert!(editor.key(key));
        assert_eq!(editor.state(), canon("p a $xy$| b"), "{key}");
        assert_eq!((source(&editor), editor.marks().contains(&Mark::Math)), (None, false), "{key}");
        editor.type_text("!");
        assert_eq!(editor.state(), canon("p a $xy$!| b"), "typing after {key}");
    }
    // Away from a formula the key is not the editor's.
    assert!(!Harness::new("p a $xy$ |b").key("Escape"));
}

#[test]
fn a_selection_inside_a_formula_is_in_it() {
    let cases = [
        ("p a $x^y|z$ b", Some("xyz")),
        ("p a $^xyz|$ b", Some("xyz")),
        ("p a^ $xy|z$ b", None),
        ("formula a^b|c", Some("abc")),
        ("formula a^bc\nformula d|", None),
    ];
    for (state, inside) in cases {
        assert_eq!(source(&Harness::new(state)), inside.map(str::to_owned), "{state:?}");
    }
}

#[test]
fn a_paste_into_a_formula_is_plain_text() {
    for (into, after) in [("p $x|$", "p $x\\*\\*b\\*\\*|$"), ("formula x|", "formula x\\*\\*b\\*\\*|")] {
        let mut editor = Harness::new(into);
        editor.platform.clip = Some(clip("**b**", Some("<b>b</b>")));
        editor.key("Mod+v");
        assert_eq!(editor.state(), canon(after), "into {into:?}");
    }
}
