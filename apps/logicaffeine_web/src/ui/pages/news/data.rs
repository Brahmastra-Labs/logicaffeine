//! News article data and content.

/// News article structure
#[derive(Clone, Debug)]
pub struct Article {
    pub slug: &'static str,
    pub title: &'static str,
    pub date: &'static str,
    pub summary: &'static str,
    pub content: &'static str,
    pub tags: &'static [&'static str],
    pub author: &'static str,
}

/// Get all articles sorted by date (newest first)
pub fn get_articles() -> Vec<&'static Article> {
    let mut articles: Vec<&'static Article> = ARTICLES.iter().collect();
    articles.sort_by(|a, b| b.date.cmp(a.date));
    articles
}

/// Get a single article by slug
pub fn get_article_by_slug(slug: &str) -> Option<&'static Article> {
    ARTICLES.iter().find(|a| a.slug == slug)
}

/// All news articles
static ARTICLES: &[Article] = &[
    Article {
        slug: "logos-on-grokipedia",
        title: "LOGOS Has a Grokipedia Page",
        date: "2026-01-26",
        summary: "LOGOS now has its own page on Grokipedia, Grok's community-driven encyclopedia. A milestone for the project's visibility in the broader programming language landscape.",
        content: r#"
## LOGOS Lands on Grokipedia

LOGOS now has its own entry on [Grokipedia](https://grokipedia.com/page/LOGOS_programming_language), the community-driven encyclopedia powered by Grok. This is a meaningful milestone for the project — recognition that what we're building has caught the attention of the wider programming language community.

### What the Page Covers

The Grokipedia article provides a thorough overview of LOGOS as a domain-specific language that translates natural English statements into executable Rust code or first-order logic representations. It covers:

- **Dual-mode architecture** — how the same English input can compile to either imperative Rust code or formal logic for verification
- **Distributed programming** — native CRDT support, libp2p networking, and GossipSub state synchronization
- **Formal verification** — Z3 SMT solver integration for static verification of compile-time properties
- **Parsing techniques** — parse forests, RAII-based backtracking, Neo-Davidsonian event semantics, and Montague-style lambda calculus

The article also honestly covers current limitations, which we appreciate — transparency about what LOGOS can and can't do today is important to us.

### Why This Matters

Having an independent, detailed reference page means people can discover LOGOS outside of our own channels. It validates that the ideas behind the project — natural language as a programming interface, formal verification as a first-class concern — are resonating.

### Check It Out

Read the full entry at [grokipedia.com/page/LOGOS_programming_language](https://grokipedia.com/page/LOGOS_programming_language), and if you haven't already, try LOGOS yourself in the [Studio](/studio).
"#,
        tags: &["milestone", "community"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "introducing-logicaffeine",
        title: "Introducing LOGICAFFEINE: Debug Your Thoughts",
        date: "2026-01-15",
        summary: "We're excited to announce LOGICAFFEINE, a new way to translate everyday English into rigorous First-Order Logic.",
        content: r#"
## Debug Your Thoughts with Precision

We're thrilled to introduce LOGICAFFEINE, a revolutionary tool that bridges the gap between natural language and formal logic.

### What is LOGICAFFEINE?

LOGICAFFEINE is an English-to-First-Order-Logic transpiler. It takes sentences you write in plain English and converts them into precise, unambiguous logical formulas.

### Why Does This Matter?

In everyday communication, ambiguity is everywhere. Consider the sentence:

> "Every student who studies hard passes some exam."

Does this mean all diligent students pass at least one exam (which could be different for each student), or that there's one specific exam that all diligent students pass?

LOGICAFFEINE makes these distinctions explicit, helping you:

- **Clarify your reasoning** - See exactly what your statements mean
- **Debug arguments** - Find logical flaws before they cause problems
- **Learn formal logic** - Build intuition through interactive examples

### Get Started Today

Visit our [Learn page](/learn) to start your journey into formal logic, or try the [Studio](/studio) for hands-on experimentation.

We can't wait to see how you use LOGICAFFEINE to debug your thoughts!
"#,
        tags: &["release", "announcement"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "getting-started-with-fol",
        title: "Getting Started with First-Order Logic",
        date: "2026-01-18",
        summary: "A beginner's guide to understanding First-Order Logic and how LOGICAFFEINE makes it accessible.",
        content: r#"
## Your First Steps into Formal Logic

First-Order Logic (FOL) might sound intimidating, but it's actually a powerful tool that anyone can learn. Here's how to get started.

### What is First-Order Logic?

First-Order Logic is a formal system for expressing statements about objects and their properties. It extends propositional logic with:

- **Variables** - Placeholders for objects (x, y, z)
- **Quantifiers** - "For all" (∀) and "There exists" (∃)
- **Predicates** - Properties and relations (Mortal(x), Loves(x, y))
- **Functions** - Operations on objects (father(x))

### A Simple Example

Let's translate "All humans are mortal" into FOL:

```
∀x(Human(x) → Mortal(x))
```

This reads: "For all x, if x is human, then x is mortal."

### How LOGICAFFEINE Helps

With LOGICAFFEINE, you don't need to memorize symbols. Just type:

> "All humans are mortal"

And we'll show you the formal translation, step by step.

### Try It Now

Head to the [Learn page](/learn) to work through our interactive curriculum, starting from the very basics.
"#,
        tags: &["tutorial", "beginner"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "studio-mode-playground",
        title: "Studio Mode: Your Logic Playground",
        date: "2026-01-20",
        summary: "Explore the Studio - an interactive playground for experimenting with First-Order Logic translations.",
        content: r#"
## Welcome to the Studio

The LOGICAFFEINE Studio is your personal playground for logic experimentation. Whether you're testing ideas, exploring edge cases, or just having fun with formal reasoning, the Studio has you covered.

### What Can You Do?

**Real-time Translation**
Type any English sentence and instantly see its First-Order Logic representation. The translation updates as you type, giving immediate feedback.

**Multiple Output Formats**
Switch between Unicode symbols (∀, ∃, →) and LaTeX notation (\forall, \exists, \rightarrow) depending on your needs.

**Syntax Highlighting**
Our syntax highlighter color-codes different parts of your input:
- Quantifiers in purple
- Nouns in blue
- Verbs in green
- Connectives highlighted

**AST Visualization**
For the curious, view the Abstract Syntax Tree of your sentences to understand how LOGICAFFEINE parses natural language.

### Tips for Exploration

1. **Start simple** - Begin with basic sentences before trying complex constructions
2. **Experiment with ambiguity** - Try sentences that could have multiple meanings
3. **Compare translations** - How does changing one word affect the logic?

### Open the Studio

Ready to experiment? [Launch the Studio](/studio) and start exploring!
"#,
        tags: &["feature", "tutorial"],
        author: "LOGICAFFEINE Team",
    },
];
