/**
 * What a heal is, apart from the wire: the request's shape, the prompt the model gets, and how its
 * answer is read. Pure functions, so `heal.test.js` holds every rule without a worker around it.
 */

/** The Studio modes that heal: a person's English that must become LOGOS, or logic sentences. */
export const MODES = ['code', 'logic'];

/** The most text one heal takes, in UTF-8 bytes. */
import { PRIMER } from './primer.js';

export const MAX_TEXT_BYTES = 8192;

/**
 * The body as the worker reads it, or why it is refused: `{ valid, mode, text }` or
 * `{ valid: false, error }` with one of `unknown_mode`, `text_missing`, `text_too_long`. The text is
 * kept as written: a heal that trimmed or normalised would change what the reader sees as its own.
 */
export function validateRequest(body, maxBytes) {
  const mode = body?.mode;
  if (typeof mode !== 'string' || !MODES.includes(mode)) return { valid: false, error: 'unknown_mode' };
  const text = body?.text;
  if (typeof text !== 'string' || text.trim() === '') return { valid: false, error: 'text_missing' };
  if (new TextEncoder().encode(text).byteLength > maxBytes) return { valid: false, error: 'text_too_long' };
  return { valid: true, mode, text, fragment: body.fragment === true };
}

const CODE_TASK = [
  'You mend LOGOS programs. LOGOS is the English programming language from logicaffeine.com: every',
  'line is an English sentence, and the program is built from the sentences as written.',
  'A person wrote the program below in a hurry, by hand. Rewrite it as clear, idiomatic, optimized',
  'LOGOS: fix typos and misspelled words, mend sentences the language would not take, name things',
  'plainly, and keep the structure the person chose. Keep every meaning: a misspelled word becomes',
  'the word the person meant ("is at leest 90" becomes "is at least 90", "Shwo" becomes "Show"), and',
  'a comparison, a number, a name or',
  'a string stays what it was. Change nothing that is already right. Keep headings, comments and',
  'blank lines where they are. Every statement begins with its statement word, capitalized (Let,',
  'Set, Show, Return, If, While, Repeat, Push, Give); a sentence that is not one of the forms below',
  'must become one, as in the examples.',
].join(' ');

const WHOLE_PROGRAM = 'Answer with the whole program and nothing else: no explanation, no greeting, no code fence.';

/** Loose English and the LOGOS it becomes: the few worked examples a small model mends by. */
const CODE_EXAMPLES = [
  'Examples.',
  'Before: make a list of the numbers 1 to 5 called nums',
  '        for each number print it doubled',
  'After:  Let nums be [1, 2, 3, 4, 5].',
  '        Repeat for n in nums:',
  '            Show n * 2.',
  'Before: give back "Hello, " + name',
  'After:  Return "Hello, " combined with name.',
  'Before: if score is at leest 90 then shwo grade A',
  'After:  If score is at least 90:',
  '            Show "Grade: A".',
  'Before: set up a counter at zero and count to three printing each',
  'After:  Let mutable counter be 0.',
  '        While counter is less than 3:',
  '            Increase counter by 1.',
  '            Show counter.',
  'Before: if who is empty say hello stranger else say hello to who',
  'After:  If who is equal to "":',
  '            Show "Hello, stranger.".',
  '        Otherwise:',
  '            Show "Hello, " combined with who.',
  'A variable is declared with Let before Set changes it; the other branch is Otherwise, never Else.',
].join('\n');

const LOGIC_TASK = [
  'You mend sentences for a logic reader. The reader at logicaffeine.com turns English into',
  'first-order logic, one sentence per line: quantifiers (every, some, no), connectives (and, or,',
  'not, if then), and plain predicates. A person wrote the lines below in a hurry, by hand.',
  'Rewrite them as clear English sentences the reader takes: fix typos, give each sentence its own',
  'line, make every quantifier and connective explicit, and keep every meaning: a misspelled word',
  'becomes the word the person meant, and a name or a number stays what it was. Change nothing that',
  'is already right. Keep headings, comments and blank lines where they are.',
  'Answer with the whole text and nothing else: no explanation, no greeting, no code fence.',
].join(' ');

const FRAGMENT_NOTE = [
  'The text is a fragment: the lines a person just wrote, inside a program that is already right',
  'and is not shown. Mend only these lines, keep their indentation exactly, add no heading and no',
  'other lines, and answer with the mended lines only.',
].join(' ');

/**
 * The system line for `mode`, and the person's text as the one user turn, quoted unchanged. A
 * `fragment` is the lines a person just wrote inside a program the worker does not see: those
 * lines come back mended, and nothing else.
 */
export function promptFor(mode, text, fragment = false) {
  const task = mode === 'logic' ? LOGIC_TASK : `${CODE_TASK} ${fragment ? '' : WHOLE_PROGRAM}\n\n${CODE_EXAMPLES}`;
  const scope = fragment ? `\n\n${FRAGMENT_NOTE}` : '';
  return { system: `${task}${scope}\n\nLOGOS on one page, the forms to write:\n${PRIMER}`, user: text };
}

const FENCE = /^\s*```[a-zA-Z0-9_-]*\n([\s\S]*?)\n?```\s*$/;
const NOT_TEXT = /^(i'm sorry|i am sorry|sorry,|i can(not|'t)|could you|can you|what do you mean|please clarify)/i;

/**
 * The healed text from the model's chat-completion answer: the first choice's content, a code
 * fence shed if one came back, or why there is none: `empty_answer` for nothing, `no_text` for a
 * refusal or a question in place of the text.
 */
export function healedFrom(response) {
  const content = response?.choices?.[0]?.message?.content;
  if (typeof content !== 'string') return { ok: false, error: 'empty_answer' };
  const fenced = content.match(FENCE);
  const healed = fenced ? `${fenced[1]}\n` : content;
  if (healed.trim() === '') return { ok: false, error: 'empty_answer' };
  if (NOT_TEXT.test(healed.trim())) return { ok: false, error: 'no_text' };
  return { ok: true, healed };
}
