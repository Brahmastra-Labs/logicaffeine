#!/usr/bin/env python3
"""
Curriculum Transformer - Council-Approved Pedagogical Updates

This script transforms exercises to meet the Pedagogical Council's standards:
1. Replace null hints with Socratic questions
2. Replace boilerplate explanations with specific, helpful content
3. Add visual_concept fields where appropriate
"""

import json
import os
import re
from pathlib import Path

# Modal Logic Hint/Explanation Templates based on prompt patterns
MODAL_TRANSFORMS = {
    # Pattern: "couldn't be" / "couldn't have" - impossibility
    r"couldn't be|couldn't have": {
        "hint": "Does 'couldn't' mean 'not possible' or 'not necessary'? Think about what's being ruled out.",
        "explanation": "'Couldn't be X' means 'it's NOT POSSIBLE to be X' — the door to that possibility is locked shut. We write this as ~◇P (not-possible P). This is different from '◇~P' (possibly not P), which only says there's SOME world where P is false."
    },
    # Pattern: "consistent with"
    r"consistent with": {
        "hint": "When two things are 'consistent', does that mean they ARE both true, or they COULD be true together?",
        "explanation": "Consistency means: there exists at least one possible world where both statements are true together. We express this with ◇(A · B) — 'it's POSSIBLE that A and B'. This doesn't claim they're actually true, just that they don't contradict each other."
    },
    # Pattern: "isn't possible" / "not possible"
    r"isn't possible|not possible|impossible": {
        "hint": "If something 'isn't possible', are we saying it fails in SOME worlds or in ALL worlds?",
        "explanation": "'Not possible' means there's NO world where this is true — every door is locked. We write ~◇P. This is equivalent to saying 'necessarily NOT P' (□~P) — it must be false everywhere."
    },
    # Pattern: "isn't necessary" / "not necessary"
    r"isn't necessary|not necessary|needn't": {
        "hint": "If something 'isn't necessary', does that mean it's false, or just that it doesn't HAVE to be true?",
        "explanation": "'Not necessary' means: there's at least one possible world where this is false. We write ~□P. This is weaker than saying 'P is false' — it just means P isn't guaranteed. In fact, P might still be true in the actual world!"
    },
    # Pattern: "must be" / "it must be that"
    r"must be|it must be": {
        "hint": "Does 'must' attach to the whole statement, or to part of it? What exactly is being required?",
        "explanation": "'Must be' expresses necessity — □ (the box). What follows 'must' is what's necessarily true. Pay attention to negations: 'must not be X' is □~X (necessarily not X), which is different from ~□X (not necessarily X)."
    },
    # Pattern: "entails" / "entailment"
    r"entails|entailment|logically implies": {
        "hint": "Does 'A entails B' mean 'if A then B' is TRUE, or that 'if A then B' is NECESSARY?",
        "explanation": "Entailment is NECESSARY implication: in EVERY possible world, if A is true then B must also be true. We write □(A ⊃ B). This is stronger than just 'A ⊃ B', which only claims the implication holds in the actual world."
    },
    # Pattern: "self-contradictory"
    r"self-contradictory|contradicts itself": {
        "hint": "If something 'contradicts itself', can it be true in ANY possible world?",
        "explanation": "A self-contradictory statement is impossible — there's no world where it's true. We express this as ~◇P (not possible). Equivalently, its negation is necessary: □~P."
    },
    # Pattern: "If X is necessary then Y is necessary"
    r"If .+ is necessary.+then.+is necessary": {
        "hint": "Is the 'if...then' itself necessary, or is it a regular conditional connecting two necessity claims?",
        "explanation": "This connects two necessity statements with a regular conditional: 'IF (□A) THEN (□B)'. Each □ wraps its own proposition. Compare to □(A ⊃ B) which would make the implication itself necessary."
    },
    # Pattern: "Necessarily, if X then Y"
    r"Necessarily, if|It's necessary that if": {
        "hint": "Is the necessity wrapping the ENTIRE if-then, or just part of it?",
        "explanation": "When 'necessarily' comes before 'if...then', the whole conditional is necessary: □(A ⊃ B). This means: in EVERY possible world, if A is true then B is true. The implication itself is a necessary truth."
    },
    # Pattern: "If X, then it's necessary that Y"
    r"if .+, then it's necessary|if .+, then .+ must": {
        "hint": "Does the necessity attach to the consequence only, or to the whole statement?",
        "explanation": "Here the 'if...then' is a regular conditional, but its CONSEQUENCE involves necessity: A ⊃ □B. This says: 'If A happens, then B becomes necessary.' The implication itself might be contingent."
    },
    # Pattern: "could be" / "might be" - possibility
    r"could be|might be|possibly": {
        "hint": "Does 'could' or 'might' suggest certainty, or just an open possibility?",
        "explanation": "Possibility is expressed with ◇ (the diamond). '◇P' means: there exists at least one possible world where P is true. It doesn't claim P IS true — just that the door to P being true is unlocked."
    },
    # Pattern: "it's possible that someone/everyone"
    r"possible that (someone|everyone|all|some)": {
        "hint": "Is the possibility about a SPECIFIC person, or about the existence of such a person?",
        "explanation": "When possibility mixes with quantifiers, scope matters! '◇∃x(Px)' means 'possibly, someone is P' — there's a world where at least one P-person exists. '∃x(◇Px)' would mean 'there's an actual person who could be P' — subtly different!"
    }
}

# Propositional Logic Hint/Explanation Templates
PROPOSITIONAL_TRANSFORMS = {
    # Pattern: "either...or" with negations
    r"either not .+ or not": {
        "hint": "How many negations do you see? Does each thing get its own 'not', or is the whole statement negated?",
        "explanation": "Count the 'not's carefully. 'Either NOT A or NOT B' gives us (~A ∨ ~B) — each part is individually negated. This is very different from ~(A ∨ B), which negates the entire disjunction."
    },
    # Pattern: "both...and"
    r"both .+ and": {
        "hint": "What exactly is 'both' connecting? Are both parts the full consequence, or just the second one?",
        "explanation": "In 'both A and B', we conjoin A and B: (A · B). Watch for scope: 'If X, then both A and B' means X ⊃ (A · B). But '(If X then A), and B' means (X ⊃ A) · B — the B stands alone!"
    },
    # Pattern: "If X, then Y, and Z"
    r"[Ii]f .+, then .+, and .+": {
        "hint": "Does the 'and Z' attach to the consequence of the if-then, or is it a separate claim?",
        "explanation": "English is ambiguous here! Usually 'If X, then Y, and Z' means: (X ⊃ Y) · Z — the implication plus a separate conjunct. If Z were part of the consequence, we'd write: X ⊃ (Y · Z)."
    },
    # Pattern: "If X, then Y or Z"
    r"[Ii]f .+, then .+ or": {
        "hint": "Is the 'or' part of what follows from the 'if', or a separate alternative?",
        "explanation": "In 'If X, then Y or Z', the disjunction is the consequence: X ⊃ (Y ∨ Z). Being X leads to the choice between Y and Z. Compare to '(If X then Y) or Z' which would be (X ⊃ Y) ∨ Z."
    },
    # Pattern: "necessary and sufficient"
    r"necessary and sufficient": {
        "hint": "'Necessary AND sufficient' — is this just an 'if' or something stronger?",
        "explanation": "'A is necessary and sufficient for B' means they're equivalent: A ≡ B (biconditional). A guarantees B (A ⊃ B) AND B guarantees A (B ⊃ A). They rise and fall together."
    },
    # Pattern: "only if"
    r"only if": {
        "hint": "'A only if B' — which way does the arrow point? Is B the cause or the requirement?",
        "explanation": "'Only if' is tricky! 'A only if B' means A ⊃ B. Think: A can only happen IF B is satisfied. B is a NECESSARY condition for A. Compare to 'A if B' which would be B ⊃ A."
    },
    # Pattern: "just if" / "if and only if"
    r"just if|if and only if|iff": {
        "hint": "'Just if' means the same as 'if and only if' — what kind of conditional is that?",
        "explanation": "'A just if B' expresses the biconditional: A ≡ B. It means A and B are equivalent — each implies the other. Whenever one is true, so is the other."
    },
    # Pattern: "sufficient for"
    r"sufficient for": {
        "hint": "If A is 'sufficient' for B, does that mean A guarantees B, or B guarantees A?",
        "explanation": "'A is sufficient for B' means: having A is enough to guarantee B. So A ⊃ B. A causes B, A leads to B, A is all you need for B."
    },
    # Pattern: "necessary for"
    r"necessary for": {
        "hint": "If A is 'necessary' for B, can B happen without A?",
        "explanation": "'A is necessary for B' means: you can't have B without A. Equivalently, B ⊃ A — if B happens, A must have happened. A is required for B."
    },
    # Pattern: "Not both"
    r"[Nn]ot both": {
        "hint": "Is 'not both' saying neither is true, or just that they're not BOTH true together?",
        "explanation": "'Not both A and B' means ~(A · B). At least one is false — but one could still be true! This is weaker than 'neither A nor B' which would be (~A · ~B)."
    }
}

# Syllogistic Hint Templates
SYLLOGISTIC_TRANSFORMS = {
    # Pattern: "All X" statements
    r"^All ": {
        "hint": "Who or what is this statement ABOUT? In 'All X is Y', X is the subject being described.",
        "explanation": "Universal affirmative: every member of the subject class has the predicate property. 'All D is H' means: take any D, that D is H. We use uppercase because we're talking about CLASSES, not individuals."
    },
    # Pattern: "No X" statements
    r"^No ": {
        "hint": "Is this saying 'not all' or 'not any at all'?",
        "explanation": "Universal negative: NO member of the subject class has the predicate property. 'No X is Y' means: take any X, that X is NOT Y. This is stronger than 'Some X is not Y'."
    },
    # Pattern: "Some X" statements
    r"^Some ": {
        "hint": "Does 'some' mean 'at least one' or 'all of them'?",
        "explanation": "'Some' means 'at least one exists.' 'Some X is Y' claims there's at least one thing that's both X and Y. We don't know how many — could be one, could be all."
    },
    # Pattern: "the X-est" (superlatives)
    r"the \w+est|the most \w+|the least \w+": {
        "hint": "Does 'the X-est' pick out a category of people, or exactly ONE specific person?",
        "explanation": "Superlatives like 'the tallest' or 'the smartest' pick out a SPECIFIC INDIVIDUAL — whoever uniquely holds that title. Use lowercase. Compare to 'a tall person' which describes a category."
    },
    # Pattern: "You" as subject
    r"^You ": {
        "hint": "Does 'you' refer to people in general, or to ONE specific person (the listener)?",
        "explanation": "'You' refers to a specific individual — the person being addressed. Use lowercase 'u'. Compare: 'People' or 'Everyone' would be uppercase because they're categories."
    },
    # Pattern: "I'm" statements
    r"^I'm |^I am ": {
        "hint": "Does 'I' refer to one specific person (the speaker) or a category?",
        "explanation": "'I' always refers to a specific individual — the speaker. Use lowercase 'i'. The predicate might be a class (uppercase) or another individual (lowercase)."
    },
    # Pattern: "It is false that" / negated statements
    r"It is false that|It's false that": {
        "hint": "What is being negated here? The whole statement, or just part of it?",
        "explanation": "'It is false that...' negates whatever follows. If 'some X is not Y' is false, then ALL X must be Y. Negating particular statements gives universal ones."
    },
    # Pattern: "Whoever" / "Anyone who"
    r"Whoever|Anyone who|Everyone who": {
        "hint": "Is this about a specific person, or about EVERYONE who fits a description?",
        "explanation": "'Whoever is X...' is a universal statement about all X's. 'Whoever is childish isn't bitter' means 'No C is B' — universal negative. The 'whoever' signals we're talking about a category."
    },
    # Pattern: "unless"
    r"unless": {
        "hint": "'Unless' is tricky in logic — try rephrasing it as 'if not'. What happens then?",
        "explanation": "'A unless B' typically means 'if not B, then A' (equivalently: A or B). 'No one is X unless they're Y' means: to be X, you must be Y. All X are Y."
    }
}

# Definitions (Informal Logic) Hint Templates
DEFINITIONS_TRANSFORMS = {
    "Too broad": {
        "hint": "Can you think of something that fits this definition but ISN'T actually a {term}?",
        "explanation": "This definition captures TOO MUCH. It includes things that shouldn't be included. A good definition should exclude non-examples while including all genuine examples."
    },
    "Too narrow": {
        "hint": "Can you think of a genuine {term} that this definition would wrongly EXCLUDE?",
        "explanation": "This definition is TOO NARROW. It excludes things that should be included. Some genuine {term}s don't fit this definition, making it incomplete."
    },
    "Circular": {
        "hint": "Does this definition use the word it's trying to define, or something equivalent?",
        "explanation": "Circular definitions use the term being defined (or a close variant) in the definition itself. This doesn't help someone who doesn't already know what the term means."
    },
    "Uses poorly understood terms": {
        "hint": "Would an average person understand all the words in this definition?",
        "explanation": "Good definitions use simpler, more familiar terms than the word being defined. If the definition requires its own dictionary, it fails as a definition."
    },
    "Poor match in vagueness": {
        "hint": "Is the original term vague while the definition is precise, or vice versa?",
        "explanation": "Definitions should match the vagueness of the term. Defining a vague term with precise boundaries (or a precise term vaguely) creates a mismatch."
    },
    "Poor match in emotional tone": {
        "hint": "Does this definition describe neutrally, or does it judge/evaluate?",
        "explanation": "Good definitions match the emotional tone of the term. Defining a neutral term with loaded language (or vice versa) distorts the meaning."
    },
    "Has non-essential properties": {
        "hint": "Does this definition include features that are common but not ESSENTIAL to being a {term}?",
        "explanation": "Definitions should capture essential properties, not accidental ones. A {term} might typically have certain features without those features being required."
    }
}

# Belief/Epistemic Logic Hint Templates
BELIEF_TRANSFORMS = {
    # Pattern: "Believe that X is false"
    r"Believe that .+ is false": {
        "hint": "Who holds this belief? Look for the underline — it marks the believer. How do we symbolize believing something FALSE?",
        "explanation": "In belief logic, 'believe that P is false' becomes u:~P — the agent (underlined u) believes NOT-P. The underline marks who holds the belief, the colon connects believer to belief content."
    },
    # Pattern: "Believe that X is true"
    r"Believe that .+ is true": {
        "hint": "Who is the believer (underlined)? What do they believe?",
        "explanation": "In belief logic, 'believe that P is true' becomes u:P — the agent (underlined u) believes P. The underline marks the believer, the colon connects them to what they believe."
    },
    # Pattern: "If you believe X, then you believe Y"
    r"If you believe .+, then you believe": {
        "hint": "Is this about the RELATIONSHIP between two beliefs, or a single complex belief?",
        "explanation": "This is a CONDITIONAL connecting two belief statements: (u:X ⊃ u:Y). The 'if...then' is outside the beliefs — it says: IF the agent believes X, THEN they also believe Y."
    },
    # Pattern: "Don't want" / "Don't believe"
    r"Don't want|Don't believe": {
        "hint": "Who is the agent with this desire/belief? Where does the negation go — before the agent or after?",
        "explanation": "The negation attaches to the whole attitude: ~u:P means 'you DON'T want/believe P'. The underlined agent still marks WHO would have the attitude (if they had it)."
    },
    # Pattern: "You want everyone" / "want everyone"
    r"[Yy]ou want everyone|want all|want everybody": {
        "hint": "Is the 'everyone' INSIDE what you want (you want: everyone does X), or OUTSIDE?",
        "explanation": "When 'everyone' is inside the desire, we write u:(∀x)Px — you want it to be the case that everyone does P. The universal quantifier is inside the scope of your desire."
    },
    # Pattern: "evident to you" / "It isn't evident"
    r"evident|isn't evident|not evident": {
        "hint": "Evidentiality (O) is like belief but stronger. What is or isn't evident, and to whom?",
        "explanation": "Evidentiality uses O (for 'obvious/evident'). 'It's evident to you that P' becomes Ou:P. 'NOT evident' becomes ~Ou:P — the evidence operator is negated, not the content."
    },
    # Pattern: General "want" statements
    r"[Yy]ou want|want to|wants to": {
        "hint": "In belief/desire logic, who is the AGENT having the desire? What do they desire?",
        "explanation": "Desires work like beliefs: u:P means 'you (agent u) want P'. The underlined letter marks who has the desire. The content after the colon is what they want."
    }
}

# Deontic Logic Hint Templates
DEONTIC_TRANSFORMS = {
    # Pattern: "ought to" / "should"
    r"ought to|should": {
        "hint": "Who is the AGENT being commanded? Look for the underline — it marks who must act.",
        "explanation": "In deontic logic, 'ought' is expressed with O (obligation). The underlined letter shows the AGENT — who is being obligated. 'Tom ought to pray' becomes OP_t where the underline under t marks Tom as the agent who must pray."
    },
    # Pattern: "Don't" / "Do not"
    r"^Don't|^Do not|don't|do not": {
        "hint": "Is this a COMMAND not to do something? Who is being commanded (the agent)?",
        "explanation": "A prohibition is a command NOT to do something. The agent (who must refrain) gets underlined. 'Don't disturb Tom' means YOU (u) are commanded not to disturb — so the underline goes under u, not t."
    },
    # Pattern: "Let no one" / "No one should"
    r"[Ll]et no one|[Nn]o one should|[Nn]obody may": {
        "hint": "Is this about a specific person, or about EVERYONE being prohibited?",
        "explanation": "Universal prohibitions use quantifiers with deontic operators. 'Let no one talk' means: there should not exist anyone who talks. We negate the existential quantifier over underlined agents."
    },
    # Pattern: "It's required" / "required that"
    r"required|must|it is obligatory": {
        "hint": "What is being required, and who must do it? The underline marks the agent.",
        "explanation": "'Required' expresses obligation (O). The person who must act gets underlined. Watch the difference between 'required that you cry' (you're the agent) vs 'required that Tom cries' (Tom's the agent)."
    },
    # Pattern: "permissible" / "allowed" / "may"
    r"permissible|allowed|may|permitted": {
        "hint": "Permission means something is ALLOWED, not required. Who has this permission?",
        "explanation": "Permission uses R (sometimes P for 'permitted'). 'It's permissible for you to sell' means you're ALLOWED to sell — not that you must. The agent (you, underlined) has the permission."
    },
    # Pattern: "consistent" in deontic context
    r"consistent": {
        "hint": "Can a command and a description both be true without contradiction?",
        "explanation": "In deontic logic, we distinguish the commanded action (underlined) from the description. 'You're selling but don't sell' — the first 'selling' describes what you're doing (Su), the second is a command not to (¬S_u). These can be consistent: you CAN be doing what you shouldn't!"
    },
    # Pattern: involving "everyone" / "all"
    r"everyone|all people|everybody": {
        "hint": "When everyone must do something, does each person become an agent?",
        "explanation": "Universal obligations use quantifiers: 'Everyone ought to X' becomes (∀x)OX_x — for all x, x is obligated to X. Each person is an agent of their own obligation."
    }
}


def get_transform_for_prompt(prompt: str, module_type: str) -> dict:
    """Find the best transformation based on prompt patterns."""

    if module_type == "modal":
        transforms = MODAL_TRANSFORMS
    elif module_type == "propositional":
        transforms = PROPOSITIONAL_TRANSFORMS
    elif module_type == "syllogistic":
        transforms = SYLLOGISTIC_TRANSFORMS
    elif module_type == "deontic":
        transforms = DEONTIC_TRANSFORMS
    elif module_type == "belief":
        transforms = BELIEF_TRANSFORMS
    else:
        return None

    for pattern, transform in transforms.items():
        if re.search(pattern, prompt, re.IGNORECASE):
            return transform

    return None


def transform_belief_exercise(exercise: dict) -> dict:
    """Transform a belief/epistemic logic exercise."""

    prompt = exercise.get("prompt", "")
    transform = get_transform_for_prompt(prompt, "belief")

    if transform:
        if exercise.get("hint") is None:
            exercise["hint"] = transform["hint"]
        if "subscript indicates who holds" in exercise.get("explanation", ""):
            exercise["explanation"] = transform["explanation"]
    else:
        if exercise.get("hint") is None:
            exercise["hint"] = "Who is the AGENT (marked with underline)? What is the content of their belief/desire/knowledge?"

    return exercise


def transform_deontic_exercise(exercise: dict) -> dict:
    """Transform a deontic logic exercise."""

    prompt = exercise.get("prompt", "")
    transform = get_transform_for_prompt(prompt, "deontic")

    if transform:
        if exercise.get("hint") is None:
            exercise["hint"] = transform["hint"]
        if "Deontic operators express moral" in exercise.get("explanation", ""):
            exercise["explanation"] = transform["explanation"]
    else:
        if exercise.get("hint") is None:
            exercise["hint"] = "Identify WHO is being commanded or permitted (the agent, marked with underline) and WHAT action they must/may do."

    return exercise


def transform_informal_exercise(exercise: dict) -> dict:
    """Transform a definitions/informal logic exercise."""

    # For definitions, we need to identify the type of error
    options = exercise.get("options", [])
    correct_idx = exercise.get("correct", 0)

    if correct_idx < len(options):
        error_type = options[correct_idx]

        if error_type in DEFINITIONS_TRANSFORMS:
            transform = DEFINITIONS_TRANSFORMS[error_type]
            if exercise.get("hint") is None:
                # Try to extract the term being defined from the prompt
                prompt = exercise.get("prompt", "")
                # Pattern: "A X is..." or "\"X\" means..."
                match = re.search(r'^(?:A |An |The |")?(\w+)"?\s+(?:is|means|are)', prompt)
                term = match.group(1) if match else "this"
                exercise["hint"] = transform["hint"].replace("{term}", term)
    else:
        if exercise.get("hint") is None:
            exercise["hint"] = "What's wrong with this definition? Is it too inclusive, too exclusive, or flawed in some other way?"

    return exercise


def transform_modal_exercise(exercise: dict) -> dict:
    """Transform a modal logic exercise with Council-approved content."""

    prompt = exercise.get("prompt", "")

    # Find matching pattern
    transform = get_transform_for_prompt(prompt, "modal")

    if transform:
        if exercise.get("hint") is None:
            exercise["hint"] = transform["hint"]
        if "scope of modal operators" in exercise.get("explanation", ""):
            exercise["explanation"] = transform["explanation"]
    else:
        # Default transformation for unmatched patterns
        if exercise.get("hint") is None:
            exercise["hint"] = "Think about what world(s) this statement is claiming something about. Is it all worlds, some worlds, or the actual world?"

    return exercise


def transform_propositional_exercise(exercise: dict) -> dict:
    """Transform a propositional logic exercise."""

    prompt = exercise.get("prompt", "")
    transform = get_transform_for_prompt(prompt, "propositional")

    if transform:
        if exercise.get("hint") is None:
            exercise["hint"] = transform["hint"]
        if "Pay attention to the scope" in exercise.get("explanation", ""):
            exercise["explanation"] = transform["explanation"]
    else:
        if exercise.get("hint") is None:
            exercise["hint"] = "Identify the main connective first. What's the primary structure of this sentence?"

    return exercise


def transform_syllogistic_exercise(exercise: dict) -> dict:
    """Transform a syllogistic logic exercise."""

    prompt = exercise.get("prompt", "")
    transform = get_transform_for_prompt(prompt, "syllogistic")

    if transform:
        if exercise.get("hint") is None:
            exercise["hint"] = transform["hint"]
    else:
        if exercise.get("hint") is None:
            exercise["hint"] = "Identify the subject and predicate. Are they individuals (lowercase) or classes (uppercase)?"

    return exercise


def transform_file(filepath: Path, module_type: str, dry_run: bool = True) -> bool:
    """Transform a single exercise file."""

    try:
        with open(filepath, 'r') as f:
            exercise = json.load(f)
    except (json.JSONDecodeError, FileNotFoundError) as e:
        print(f"Error reading {filepath}: {e}")
        return False

    original = json.dumps(exercise, indent=2)

    if module_type == "modal":
        exercise = transform_modal_exercise(exercise)
    elif module_type == "propositional":
        exercise = transform_propositional_exercise(exercise)
    elif module_type == "syllogistic":
        exercise = transform_syllogistic_exercise(exercise)
    elif module_type == "deontic":
        exercise = transform_deontic_exercise(exercise)
    elif module_type == "informal":
        exercise = transform_informal_exercise(exercise)
    elif module_type == "belief":
        exercise = transform_belief_exercise(exercise)

    transformed = json.dumps(exercise, indent=2)

    if original != transformed:
        if dry_run:
            print(f"Would transform: {filepath}")
            print(f"  Hint: {exercise.get('hint', 'N/A')[:60]}...")
        else:
            with open(filepath, 'w') as f:
                f.write(transformed + '\n')
            print(f"Transformed: {filepath}")
        return True

    return False


def main():
    import argparse

    parser = argparse.ArgumentParser(description='Transform curriculum exercises')
    parser.add_argument('--apply', action='store_true', help='Actually apply changes (default: dry run)')
    parser.add_argument('--module', choices=['modal', 'propositional', 'syllogistic', 'deontic', 'informal', 'belief', 'all'],
                        default='all', help='Which module to transform')
    args = parser.parse_args()

    base_path = Path('/Users/collinpounds/dev/logicaffeine/assets/curriculum/00_logicaffeine')

    modules = {
        'modal': '03_modal',
        'propositional': '02_propositional',
        'syllogistic': '01_syllogistic',
        'deontic': '04_deontic',
        'belief': '05_belief',
        'informal': '06_informal'
    }

    if args.module == 'all':
        to_process = modules.items()
    else:
        to_process = [(args.module, modules[args.module])]

    total_transformed = 0

    for module_type, module_dir in to_process:
        module_path = base_path / module_dir
        if not module_path.exists():
            continue

        print(f"\n=== Processing {module_type} ({module_dir}) ===")

        for filepath in sorted(module_path.glob('*.json')):
            if filepath.name == 'meta.json':
                continue
            if transform_file(filepath, module_type, dry_run=not args.apply):
                total_transformed += 1

    print(f"\n{'Would transform' if not args.apply else 'Transformed'}: {total_transformed} files")
    if not args.apply:
        print("Run with --apply to make changes")


if __name__ == '__main__':
    main()
