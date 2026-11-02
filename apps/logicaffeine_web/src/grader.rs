use logicaffeine_language::semantics::lookup_noun_hypernyms;

#[derive(Debug, Clone)]
pub struct GradeResult {
    pub correct: bool,
    pub partial: bool,
    pub score: u32,
    pub feedback: String,
}

impl GradeResult {
    pub fn correct() -> Self {
        Self {
            correct: true,
            partial: false,
            score: 100,
            feedback: "Correct!".to_string(),
        }
    }

    pub fn partial(feedback: String, score: u32) -> Self {
        Self {
            correct: false,
            partial: true,
            score,
            feedback,
        }
    }

    pub fn incorrect(feedback: String) -> Self {
        Self {
            correct: false,
            partial: false,
            score: 0,
            feedback,
        }
    }
}

pub fn check_answer(user_input: &str, expected: &str) -> GradeResult {
    if normalize_logic(user_input) == normalize_logic(expected) {
        return GradeResult::correct();
    }

    match (Formula::read(user_input), Formula::read(expected)) {
        (Some(user), Some(expected)) => {
            if user.same_as(&expected) {
                return GradeResult::correct();
            }

            let similarity = user.similarity(&expected);
            if similarity > 0.7 {
                GradeResult::partial(
                    "Close! Check your quantifier or connective structure.".to_string(),
                    (similarity * 50.0) as u32,
                )
            } else if similarity > 0.4 {
                GradeResult::partial(
                    "Partially correct. Review the logical structure.".to_string(),
                    (similarity * 30.0) as u32,
                )
            } else {
                GradeResult::incorrect(
                    "Not quite. Consider the relationship between subject and predicate."
                        .to_string(),
                )
            }
        }
        (None, _) => {
            GradeResult::incorrect("Could not parse your answer. Check syntax.".to_string())
        }
        (_, None) => {
            GradeResult::incorrect("Internal error: could not parse expected answer.".to_string())
        }
    }
}

fn normalize_logic(input: &str) -> String {
    symbolize(input)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// The LaTeX and ASCII spellings a reader may type, as the symbols LOGOS prints.
fn symbolize(input: &str) -> String {
    let mut result = input.to_string();

    result = result.replace("\\forall", "∀");
    result = result.replace("\\exists", "∃");
    result = result.replace("\\neg", "¬");
    result = result.replace("\\land", "∧");
    result = result.replace("\\lor", "∨");
    result = result.replace("\\supset", "→");
    result = result.replace("\\equiv", "↔");
    result = result.replace("\\Box", "□");
    result = result.replace("\\Diamond", "◇");

    // Order matters: the longer arrows before the shorter, and `!=` before `!`
    result = result.replace("<=>", "↔");
    result = result.replace("<->", "↔");
    result = result.replace("->", "→");
    result = result.replace(">=", "≥");
    result = result.replace("<=", "≤");
    result = result.replace("!=", "≠");
    result = result.replace("==", "=");
    result = result.replace("&", "∧");
    result = result.replace("|", "∨");
    result = result.replace("~", "¬");
    result = result.replace("!", "¬");

    result
}

/// A formula as the grader compares it. [`Formula::read`] names each bound variable by how
/// many binders enclose it, lower-cases free names, and flattens ∧ and ∨ into their operands,
/// so formulas that differ only in a bound variable's name, a name's case, or the order and
/// grouping of their conjuncts compare equal.
#[derive(Debug, Clone)]
enum Formula {
    /// ∀, ∃, a counting quantifier (`∃=2`), a generalised one (`MOST`), or λ.
    Quantifier {
        kind: String,
        var: String,
        body: Box<Formula>,
    },
    /// ¬, or a modal (□, ◇); a modal's degree (`◇_{0.5}`) is not compared.
    Unary { op: String, body: Box<Formula> },
    /// →, ↔, or the counterfactual □→.
    Binary {
        op: String,
        left: Box<Formula>,
        right: Box<Formula>,
    },
    /// ∧ or ∨ over two or more operands, in any order.
    Junction { op: String, items: Vec<Formula> },
    /// A predicate, a relation, or an operator over formulas (`HAB(…)`) applied to its arguments.
    Apply { name: String, args: Vec<Formula> },
    /// A constant, or a variable bound above it.
    Name(String),
}

/// The generalised quantifiers LOGOS prints as a word: `MOST x(…)`.
const GENERALISED_QUANTIFIERS: [&str; 3] = ["MOST", "MANY", "FEW"];

/// The deontic operators LOGOS prints before a formula: `O_{0.6} φ` for "should", `P_{0.5} φ` for
/// "may". Read as the operator applied to φ, as `O(φ)` is.
const DEONTIC_OPERATORS: [&str; 2] = ["O", "P"];

impl Formula {
    /// The formula `text` writes, or `None` when it is not one.
    fn read(text: &str) -> Option<Formula> {
        let mut parser = Parser {
            tokens: tokens(&symbolize(text)),
            at: 0,
        };
        let formula = parser.formula()?;
        (parser.at == parser.tokens.len()).then(|| formula.canonical(&mut Vec::new()))
    }

    fn canonical(self, bound: &mut Vec<String>) -> Formula {
        match self {
            Formula::Quantifier { kind, var, body } => {
                let level = format!("#{}", bound.len());
                bound.push(var);
                let body = body.canonical(bound);
                bound.pop();
                Formula::Quantifier {
                    kind,
                    var: level,
                    body: Box::new(body),
                }
            }
            Formula::Unary { op, body } => Formula::Unary {
                op,
                body: Box::new(body.canonical(bound)),
            },
            Formula::Binary { op, left, right } => Formula::Binary {
                op,
                left: Box::new(left.canonical(bound)),
                right: Box::new(right.canonical(bound)),
            },
            Formula::Junction { op, items } => {
                let mut flat = Vec::new();
                for item in items {
                    match item.canonical(bound) {
                        Formula::Junction { op: inner, items } if inner == op => flat.extend(items),
                        other => flat.push(other),
                    }
                }
                if op == "∧" {
                    flat = without_categories(flat);
                }
                if flat.len() == 1 {
                    flat.remove(0)
                } else {
                    Formula::Junction { op, items: flat }
                }
            }
            Formula::Apply { name, args } => Formula::Apply {
                name,
                args: args.into_iter().map(|arg| arg.canonical(bound)).collect(),
            },
            Formula::Name(name) => match bound.iter().rposition(|var| *var == name) {
                Some(level) => Formula::Name(format!("#{level}")),
                None => Formula::Name(name.to_lowercase()),
            },
        }
    }

    fn same_as(&self, other: &Formula) -> bool {
        match (self, other) {
            (
                Formula::Quantifier {
                    kind: k1,
                    var: v1,
                    body: b1,
                },
                Formula::Quantifier {
                    kind: k2,
                    var: v2,
                    body: b2,
                },
            ) => k1 == k2 && v1 == v2 && b1.same_as(b2),
            (Formula::Unary { op: o1, body: b1 }, Formula::Unary { op: o2, body: b2 }) => {
                o1 == o2 && b1.same_as(b2)
            }
            (
                Formula::Binary {
                    op: o1,
                    left: l1,
                    right: r1,
                },
                Formula::Binary {
                    op: o2,
                    left: l2,
                    right: r2,
                },
            ) => {
                o1 == o2
                    && ((l1.same_as(l2) && r1.same_as(r2))
                        || (o1 == "↔" && l1.same_as(r2) && r1.same_as(l2)))
            }
            (Formula::Junction { op: o1, items: i1 }, Formula::Junction { op: o2, items: i2 }) => {
                o1 == o2
                    && i1.len() == i2.len()
                    && max_matching(i1, i2, |a, b| a.same_as(b)) == i1.len()
            }
            (Formula::Apply { name: n1, args: a1 }, Formula::Apply { name: n2, args: a2 }) => {
                n1 == n2 && a1.len() == a2.len() && a1.iter().zip(a2).all(|(x, y)| x.same_as(y))
            }
            (Formula::Name(a), Formula::Name(b)) => a == b,
            _ => false,
        }
    }

    /// How alike two formulas are, from 0 to 1, for partial credit.
    fn similarity(&self, other: &Formula) -> f64 {
        match (self, other) {
            (
                Formula::Quantifier {
                    kind: k1, body: b1, ..
                },
                Formula::Quantifier {
                    kind: k2, body: b2, ..
                },
            ) => {
                let kind_match = if k1 == k2 { 0.4 } else { 0.0 };
                kind_match + b1.similarity(b2) * 0.6
            }
            (Formula::Unary { op: o1, body: b1 }, Formula::Unary { op: o2, body: b2 }) => {
                let op_match = if o1 == o2 { 0.3 } else { 0.0 };
                op_match + b1.similarity(b2) * 0.7
            }
            (Formula::Apply { name: n1, args: a1 }, Formula::Apply { name: n2, args: a2 }) => {
                let name_match = if n1 == n2 { 0.7 } else { 0.0 };
                let arity_match = if a1.len() == a2.len() { 0.3 } else { 0.0 };
                name_match + arity_match
            }
            (Formula::Name(a), Formula::Name(b)) => {
                if a == b {
                    1.0
                } else {
                    0.0
                }
            }
            _ => match (self.connective(), other.connective()) {
                (Some((o1, c1)), Some((o2, c2))) => {
                    let op_match = if o1 == o2 { 0.3 } else { 0.0 };
                    let unordered = matches!(self, Formula::Junction { .. })
                        && matches!(other, Formula::Junction { .. });
                    let total: f64 = if unordered {
                        c1.iter()
                            .map(|a| c2.iter().map(|b| a.similarity(b)).fold(0.0, f64::max))
                            .sum()
                    } else {
                        c1.iter().zip(&c2).map(|(a, b)| a.similarity(b)).sum()
                    };
                    op_match + total / c1.len().max(c2.len()) as f64 * 0.7
                }
                _ => 0.0,
            },
        }
    }

    /// A binary connective or a junction, as its operator and operands.
    fn connective(&self) -> Option<(&str, Vec<&Formula>)> {
        match self {
            Formula::Binary { op, left, right } => Some((op, vec![left.as_ref(), right.as_ref()])),
            Formula::Junction { op, items } => Some((op, items.iter().collect())),
            _ => None,
        }
    }
}

/// The conjuncts without the dictionary categories of their siblings: beside `Bird(x)`,
/// `Animal(x)` says nothing more, because the dictionary makes every bird an animal. LOGOS
/// writes the categories out; a textbook answer leaves them implicit; both say the same.
fn without_categories(items: Vec<Formula>) -> Vec<Formula> {
    let names_a_category = |item: &Formula| {
        let Formula::Apply { name, args } = item else {
            return false;
        };
        items.iter().any(|noun| match noun {
            Formula::Apply {
                name: noun_name,
                args: noun_args,
            } => {
                noun_name != name
                    && noun_args.len() == args.len()
                    && noun_args.iter().zip(args).all(|(a, b)| a.same_as(b))
                    && lookup_noun_hypernyms(noun_name)
                        .iter()
                        .any(|category| category.eq_ignore_ascii_case(name))
            }
            _ => false,
        })
    };
    let keep: Vec<bool> = items.iter().map(|item| !names_a_category(item)).collect();
    items
        .into_iter()
        .zip(keep)
        .filter_map(|(item, keep)| keep.then_some(item))
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    /// ∀ or ∃, with its count when it has one (`∃=2`, `∃≥3`).
    Quantifier(String),
    Word(String),
    Symbol(char),
}

fn tokens(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let word_char = |c: char| c.is_alphanumeric() || matches!(c, '_' | '\'' | '-');
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '∀' || c == '∃' {
            let mut kind = c.to_string();
            i += 1;
            if let Some(&bound) = chars.get(i).filter(|b| matches!(b, '=' | '≥' | '≤')) {
                let digits: String = chars[i + 1..]
                    .iter()
                    .take_while(|d| d.is_ascii_digit())
                    .collect();
                if !digits.is_empty() {
                    kind.push(bound);
                    kind.push_str(&digits);
                    i += 1 + digits.len();
                }
            }
            out.push(Token::Quantifier(kind));
        } else if c == '□' || c == '◇' {
            out.push(Token::Symbol(c));
            i += 1;
            if chars.get(i) == Some(&'_') {
                i += 1;
                if chars.get(i) == Some(&'{') {
                    while i < chars.len() && chars[i] != '}' {
                        i += 1;
                    }
                    i += 1;
                } else {
                    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '.') {
                        i += 1;
                    }
                }
            }
        } else if word_char(c) {
            let start = i;
            while i < chars.len() {
                let decimal_point = chars[i] == '.'
                    && chars[i - 1].is_ascii_digit()
                    && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit());
                if !(word_char(chars[i]) || decimal_point) {
                    break;
                }
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            // A deontic operator's strength (`O_{0.6}`) is not compared, as a modal's is not.
            if let Some(operator) = word
                .strip_suffix('_')
                .filter(|w| DEONTIC_OPERATORS.contains(w))
            {
                if chars.get(i) == Some(&'{') {
                    while i < chars.len() && chars[i] != '}' {
                        i += 1;
                    }
                    i += 1;
                    out.push(Token::Word(operator.to_string()));
                    continue;
                }
            }
            out.push(Token::Word(word));
        } else {
            out.push(Token::Symbol(c));
            i += 1;
        }
    }
    out
}

/// Recursive descent over the notation LOGOS prints, loosest-binding first:
/// ↔, then → (right-associative, with □→), then ∨, then ∧, then ¬, modals and binders.
struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self, ahead: usize) -> Option<&Token> {
        self.tokens.get(self.at + ahead)
    }

    fn eat(&mut self, symbol: char) -> bool {
        let found = self.peek(0) == Some(&Token::Symbol(symbol));
        if found {
            self.at += 1;
        }
        found
    }

    fn formula(&mut self) -> Option<Formula> {
        let mut left = self.implication()?;
        while self.eat('↔') {
            let right = self.implication()?;
            left = Formula::Binary {
                op: "↔".to_string(),
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Some(left)
    }

    fn implication(&mut self) -> Option<Formula> {
        let left = self.junction('∨')?;
        let op = if self.eat('→') {
            "→"
        } else if self.peek(0) == Some(&Token::Symbol('□'))
            && self.peek(1) == Some(&Token::Symbol('→'))
        {
            self.at += 2;
            "□→"
        } else {
            return Some(left);
        };
        let right = self.implication()?;
        Some(Formula::Binary {
            op: op.to_string(),
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    fn junction(&mut self, op: char) -> Option<Formula> {
        let operand = |parser: &mut Parser| {
            if op == '∨' {
                parser.junction('∧')
            } else {
                parser.unary()
            }
        };
        let mut items = vec![operand(self)?];
        while self.eat(op) {
            items.push(operand(self)?);
        }
        Some(if items.len() == 1 {
            items.remove(0)
        } else {
            Formula::Junction {
                op: op.to_string(),
                items,
            }
        })
    }

    fn unary(&mut self) -> Option<Formula> {
        match self.peek(0)?.clone() {
            Token::Symbol(op @ ('¬' | '□' | '◇')) => {
                self.at += 1;
                Some(Formula::Unary {
                    op: op.to_string(),
                    body: Box::new(self.unary()?),
                })
            }
            Token::Quantifier(kind) => {
                self.at += 1;
                self.binder(kind)
            }
            Token::Symbol('λ') => {
                self.at += 1;
                self.binder("λ".to_string())
            }
            Token::Word(word)
                if GENERALISED_QUANTIFIERS.contains(&word.as_str())
                    && matches!(self.peek(1), Some(Token::Word(_)))
                    && self.peek(2) == Some(&Token::Symbol('(')) =>
            {
                self.at += 1;
                self.binder(word)
            }
            Token::Symbol('(') => {
                self.at += 1;
                let inner = self.formula()?;
                self.eat(')').then_some(inner)
            }
            Token::Word(word)
                if DEONTIC_OPERATORS.contains(&word.as_str()) && self.starts_formula(1) =>
            {
                self.at += 1;
                let body = self.unary()?;
                Some(Formula::Apply {
                    name: word,
                    args: vec![body],
                })
            }
            Token::Word(_) => self.atom(),
            _ => None,
        }
    }

    /// Whether the token `ahead` places on starts a formula an operator can govern: a binder, a
    /// negation or modal, or a predicate applied to arguments. A parenthesis does not count: `O(φ)`
    /// is read as an application, which comes to the same thing.
    fn starts_formula(&self, ahead: usize) -> bool {
        match self.peek(ahead) {
            Some(Token::Quantifier(_)) | Some(Token::Symbol('¬' | '□' | '◇' | 'λ')) => true,
            Some(Token::Word(word)) => {
                DEONTIC_OPERATORS.contains(&word.as_str())
                    || self.peek(ahead + 1) == Some(&Token::Symbol('('))
            }
            _ => false,
        }
    }

    /// A binder's variable, then its scope: the parenthesised group right after it, or else
    /// everything to its right (`∀x. φ`, `∀x φ`, `λx.φ`).
    fn binder(&mut self, kind: String) -> Option<Formula> {
        self.eat('.');
        let Some(Token::Word(var)) = self.peek(0).cloned() else {
            return None;
        };
        self.at += 1;
        let dotted = self.eat('.');
        let body = if !dotted && self.peek(0) == Some(&Token::Symbol('(')) {
            self.unary()?
        } else {
            self.formula()?
        };
        Some(Formula::Quantifier {
            kind,
            var,
            body: Box::new(body),
        })
    }

    /// A name or an application, and the relation it stands in when one follows (`x = y`).
    fn atom(&mut self) -> Option<Formula> {
        let Some(Token::Word(name)) = self.peek(0).cloned() else {
            return None;
        };
        self.at += 1;
        let term = if self.eat('(') {
            let mut args = Vec::new();
            if !self.eat(')') {
                loop {
                    args.push(self.formula()?);
                    if self.eat(')') {
                        break;
                    }
                    if !self.eat(',') {
                        return None;
                    }
                }
            }
            Formula::Apply { name, args }
        } else {
            Formula::Name(name)
        };
        for relation in ['=', '≠', '≥', '≤', '<', '>'] {
            if self.eat(relation) {
                let right = self.atom()?;
                return Some(Formula::Apply {
                    name: relation.to_string(),
                    args: vec![term, right],
                });
            }
        }
        Some(term)
    }
}

/// How a set of answers covers a sentence's readings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingsGrade {
    /// Readings matched by a distinct answer.
    pub matched: usize,
    /// Readings the sentence has.
    pub total: usize,
}

impl ReadingsGrade {
    /// Every reading given, each by its own answer. A sentence with no readings is never correct.
    pub fn all_correct(&self) -> bool {
        self.total > 0 && self.matched == self.total
    }
}

/// Grade an ambiguous sentence: every reading must be given by a different answer, in any order.
/// Each answer is graded against each reading exactly as a translation is ([`check_answer`]).
pub fn check_readings(answers: &[String], readings: &[String]) -> ReadingsGrade {
    let matched = match_readings(answers, readings, |answer, reading| {
        !answer.trim().is_empty() && check_answer(answer, reading).correct
    });
    ReadingsGrade {
        matched,
        total: readings.len(),
    }
}

/// The largest set of (answer, reading) pairs where each answer and each reading is used at
/// most once and every pair `fits` — a maximum bipartite matching by augmenting paths, so an
/// answer that fits several readings never strands another answer that fits only one.
fn match_readings(
    answers: &[String],
    readings: &[String],
    fits: impl Fn(&str, &str) -> bool,
) -> usize {
    max_matching(answers, readings, |answer, reading| fits(answer, reading))
}

/// The largest set of (left, right) pairs where each side is used at most once and every pair
/// `fits`, found by augmenting paths.
fn max_matching<L, R>(left: &[L], right: &[R], fits: impl Fn(&L, &R) -> bool) -> usize {
    let edges: Vec<Vec<usize>> = left
        .iter()
        .map(|l| (0..right.len()).filter(|&r| fits(l, &right[r])).collect())
        .collect();
    let mut owner: Vec<Option<usize>> = vec![None; right.len()];

    fn augment(
        a: usize,
        edges: &[Vec<usize>],
        owner: &mut [Option<usize>],
        seen: &mut [bool],
    ) -> bool {
        for &r in &edges[a] {
            if seen[r] {
                continue;
            }
            seen[r] = true;
            if owner[r].map_or(true, |b| augment(b, edges, owner, seen)) {
                owner[r] = Some(a);
                return true;
            }
        }
        false
    }

    (0..left.len())
        .filter(|&a| augment(a, &edges, &mut owner, &mut vec![false; right.len()]))
        .count()
}

#[cfg(test)]
#[path = "../../../crates/logicaffeine_tests/unit/logicaffeine_web/grader/tests.rs"]
mod tests;
