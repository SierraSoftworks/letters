//! Links in maths: drawn as links only where the document may link.

mod common;

use common::*;
use letters_math::{Style, typeset};

/// The inline style of a document that may link.
fn linking(size: f32) -> Style {
    Style { links: true, ..inline(size) }
}

/// Places a link can sit, with `HERE` where it goes. Between them they cover every way one piece of maths
/// holds another.
const PLACES: &[&str] = &[
    "HERE",
    "{x + HERE}",
    r"x^{HERE}_{HERE}",
    r"\frac{HERE}{HERE}",
    r"\sqrt[HERE]{HERE}",
    r"\hat{HERE} + \underline{HERE} + \overline{HERE}",
    r"\sum_{HERE}^{HERE} + \operatorname{HERE}",
    r"\mathbf{HERE} + \text{see HERE} + \textcolor{teal}{HERE} + {\color{teal} HERE}",
    r"\displaystyle HERE + \scriptstyle HERE + \Large HERE",
    r"\left( HERE \middle| HERE \right)",
    r"\mathrel{HERE} + \pmb{HERE} + \hbox{HERE} + \mathllap{HERE}",
    r"\smash{HERE} + \raisebox{1pt}{HERE} + \vcenter{HERE}",
    r"\begin{pmatrix} HERE & b \\ c & HERE \end{pmatrix}",
    r"\begin{cases} HERE & \text{if } HERE \end{cases}",
    r"\begin{equation} x = HERE \tag{HERE} \end{equation}",
    r"\underbrace{HERE}_{HERE} + \overbrace{HERE}^{HERE}",
    r"\boxed{HERE} + \cancel{HERE} + \colorbox{yellow}{HERE}",
    r"\xrightarrow[HERE]{HERE}",
    r"\mathchoice{HERE}{HERE}{HERE}{HERE}",
    r"\htmlStyle{color: teal}{HERE}",
    r"\href{https://example.org}{HERE + b}",
    r"\def\link{HERE} \link + \link",
    r"\underleftarrow{HERE}",
    r"\mathop{HERE}\limits_{x}",
    r"\phantom{HERE} + \vphantom{HERE}",
    r"\html@mathml{HERE}{HERE}",
    r"x \tag{HERE}",
    r"\begin{CD} A @>{HERE}>{HERE}> B \\ @V{HERE}VV @VVV \\ C @>>> D \end{CD}",
    r"\begin{prooftree} \AxiomC{HERE} \LeftLabel{HERE} \RightLabel{HERE} \UnaryInfC{HERE} \end{prooftree}",
];

#[test]
fn a_link_that_is_refused_is_set_as_its_plain_argument() {
    let cases = [
        (r"\href{https://example.com/page}{a}", "{a}"),
        (r"\url{https://example.com/page}", r"{\texttt{https://example.com/page}}"),
    ];
    for (link, plain) in cases {
        for place in PLACES {
            let (linked, plain) = (place.replace("HERE", link), place.replace("HERE", plain));

            assert_eq!(typeset(&linked, &inline(20.0)), typeset(&plain, &inline(20.0)), "{linked}");
            assert_ne!(typeset(&linked, &linking(20.0)), typeset(&plain, &linking(20.0)), "{linked} holds no link");
        }
    }
}

#[test]
fn a_link_is_drawn_as_one_only_where_links_are_allowed() {
    for source in [r"\href{https://example.com/page}{here}", r"\url{https://example.com/page}"] {
        let refused = render(&[set(source, &inline(40.0))], 1.0);
        let allowed = render(&[set(source, &linking(40.0))], 1.0);

        assert_eq!(darkest(&refused), INK, "{source} is set in the ink around it");
        assert_ne!(darkest(&allowed), INK, "{source} stands out as a link");
    }
}

#[test]
fn an_address_is_read_as_one_whether_or_not_it_may_link() {
    // In TeX a `%` starts a comment and `#`, `_` and `~` are special. In an address they are not.
    let source = r"\href{https://example.com/a%20b?c_d#e~f}{a} + b";

    assert_eq!(typeset(source, &inline(20.0)), typeset("{a} + b", &inline(20.0)));
    assert!(typeset(source, &linking(20.0)).is_ok());
}
