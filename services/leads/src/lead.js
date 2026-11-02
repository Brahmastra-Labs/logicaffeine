// The pricing form's field rules. The page's `lead.rs` applies the same rules, and both walk
// crates/logicaffeine_tests/tests/fixtures/lead_vectors.json, so the two validators cannot drift.

const TRIM_SET = new Set([
  0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20, 0x85, 0xa0, 0x1680,
  0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a,
  0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0xfeff,
]);

const PRINTABLE_ASCII = /^[\x21-\x7e]+$/;
const ATOM = /^[A-Za-z0-9!#$%&'*+/=?^_`{|}~-]+$/;
const LABEL = /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?$/;
const ALL_DIGITS = /^[0-9]+$/;
const PHONE_CHARACTERS = /^[0-9 +().-]+$/;
const NON_DIGITS = /[^0-9]/g;
const SOURCE = /^[A-Za-z0-9._/#-]{1,64}$/;

const MAX_EMAIL = 254;
const MAX_LOCAL = 64;
const MAX_PHONE = 32;
const MIN_DIGITS = 7;
const MAX_DIGITS = 15;

const emailError = (reason) => ({ valid: false, error: 'invalid_email', reason });
const phoneError = (reason) => ({ valid: false, error: 'invalid_phone', reason });

/**
 * `value` without leading or trailing code points from the Unicode White_Space set plus U+FEFF.
 * Every member is a single UTF-16 unit, so a scan by unit is exact.
 */
export function trimField(value) {
  let start = 0;
  let end = value.length;
  while (start < end && TRIM_SET.has(value.charCodeAt(start))) start++;
  while (end > start && TRIM_SET.has(value.charCodeAt(end - 1))) end--;
  return value.slice(start, end);
}

export function validateEmail(input) {
  if (input === null || input === undefined) return emailError('required');
  if (typeof input !== 'string') return emailError('malformed');
  const email = trimField(input);
  if (email === '') return emailError('required');
  if (email.length > MAX_EMAIL || !PRINTABLE_ASCII.test(email)) return emailError('malformed');

  const at = email.indexOf('@');
  if (at < 0 || at !== email.lastIndexOf('@')) return emailError('malformed');
  const local = email.slice(0, at);
  const domain = email.slice(at + 1);

  if (local.length > MAX_LOCAL || !local.split('.').every((atom) => ATOM.test(atom))) return emailError('malformed');

  const labels = domain.split('.');
  if (labels.length < 2 || !labels.every((label) => LABEL.test(label))) return emailError('malformed');
  const top = labels[labels.length - 1];
  if (top.length < 2 || ALL_DIGITS.test(top)) return emailError('malformed');

  return { valid: true, normalized: `${local}@${domain.toLowerCase()}` };
}

export function validatePhone(input) {
  if (input === null || input === undefined) return { valid: true, normalized: null, digits: null };
  if (typeof input !== 'string') return phoneError('malformed');
  const phone = trimField(input);
  if (phone === '') return { valid: true, normalized: null, digits: null };
  if (!PHONE_CHARACTERS.test(phone)) return phoneError('bad_character');
  if (phone.lastIndexOf('+') > 0) return phoneError('misplaced_plus');
  if (phone.length > MAX_PHONE) return phoneError('too_long');
  const digits = phone.replace(NON_DIGITS, '');
  if (digits.length < MIN_DIGITS) return phoneError('too_few_digits');
  if (digits.length > MAX_DIGITS) return phoneError('too_many_digits');
  return { valid: true, normalized: phone, digits };
}

/** The page's own tag for the form a lead came from; not user input, so it is never trimmed. */
export function validateSource(input) {
  if (input === null || input === undefined) return { valid: true, source: null };
  if (typeof input === 'string' && SOURCE.test(input)) return { valid: true, source: input };
  return { valid: false, error: 'invalid_source' };
}
