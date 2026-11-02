import { describe, expect, it } from 'vitest';
import vectors from '../../../crates/logicaffeine_tests/tests/fixtures/lead_vectors.json';
import { trimField, validateEmail, validatePhone, validateSource } from './lead.js';

const EMAIL_REASONS = ['required', 'malformed'];
const PHONE_REASONS = ['bad_character', 'misplaced_plus', 'too_long', 'too_few_digits', 'too_many_digits'];
const CASE_KEYS = {
  email: { valid: ['input', 'valid', 'normalized', 'note'], invalid: ['input', 'valid', 'error', 'reason', 'note'] },
  phone: {
    valid: ['input', 'valid', 'normalized', 'digits', 'note'],
    invalid: ['input', 'valid', 'error', 'reason', 'note'],
  },
};

function codePoints(entries) {
  const set = new Set();
  for (const entry of entries) {
    const m = /^U\+([0-9A-F]{4,6})(?:\.\.U\+([0-9A-F]{4,6}))?$/.exec(entry);
    if (!m) throw new Error(`trim_set entry ${JSON.stringify(entry)} is neither U+XXXX nor U+XXXX..U+YYYY`);
    const lo = parseInt(m[1], 16);
    const hi = parseInt(m[2] ?? m[1], 16);
    if (hi < lo) throw new Error(`trim_set range ${entry} runs backwards`);
    for (let c = lo; c <= hi; c++) set.add(c);
  }
  return set;
}

const TRIM_SET = codePoints(vectors.trim_set);

const expectedEmail = (v) =>
  v.valid ? { valid: true, normalized: v.normalized } : { valid: false, error: v.error, reason: v.reason };
const expectedPhone = (v) =>
  v.valid
    ? { valid: true, normalized: v.normalized, digits: v.digits }
    : { valid: false, error: v.error, reason: v.reason };

const isAscii = (s) => /^[\x00-\x7f]*$/.test(s);

describe('the shared vector file', () => {
  it('is version 1 and names its shape, trim set and rules', () => {
    expect(vectors.version).toBe(1);
    expect(typeof vectors.description).toBe('string');
    expect(Object.keys(vectors.shape).sort()).toEqual(['email[]', 'input_length', 'phone[]', 'trim_set']);
    expect(Object.keys(vectors.rules).sort()).toEqual(['email', 'phone', 'trim', 'worker']);
  });

  it('holds at least 40 email cases and 40 phone cases', () => {
    expect(vectors.email.length).toBeGreaterThanOrEqual(40);
    expect(vectors.phone.length).toBeGreaterThanOrEqual(40);
  });

  for (const field of ['email', 'phone']) {
    it(`gives every ${field} case exactly the keys its verdict needs`, () => {
      const wrong = vectors[field]
        .map((v) => {
          const want = [...CASE_KEYS[field][v.valid ? 'valid' : 'invalid'], ...('input_length' in v ? ['input_length'] : [])];
          return { note: v.note, have: Object.keys(v).sort(), want: want.sort() };
        })
        .filter(({ have, want }) => JSON.stringify(have) !== JSON.stringify(want));
      expect(wrong).toEqual([]);
    });

    it(`never lists a ${field} input twice`, () => {
      const seen = new Map();
      for (const v of vectors[field]) {
        const key = JSON.stringify(v.input);
        seen.set(key, (seen.get(key) ?? 0) + 1);
      }
      expect([...seen].filter(([, n]) => n > 1)).toEqual([]);
    });

    it(`has both valid and invalid ${field} cases, and a null input`, () => {
      expect(vectors[field].some((v) => v.valid)).toBe(true);
      expect(vectors[field].some((v) => !v.valid)).toBe(true);
      expect(vectors[field].some((v) => v.input === null)).toBe(true);
    });

    it(`pins every ${field} boundary case to its length, in ASCII`, () => {
      const pinned = vectors[field].filter((v) => 'input_length' in v);
      expect(pinned.length).toBeGreaterThan(0);
      for (const v of pinned) {
        expect(isAscii(v.input), v.note).toBe(true);
        expect(v.input.length, v.note).toBe(v.input_length);
      }
    });
  }

  it('exercises every email reason and names no other', () => {
    const reasons = new Set(vectors.email.filter((v) => !v.valid).map((v) => v.reason));
    expect([...reasons].sort()).toEqual([...EMAIL_REASONS].sort());
    expect(vectors.email.filter((v) => !v.valid).every((v) => v.error === 'invalid_email')).toBe(true);
  });

  it('exercises every phone reason and names no other', () => {
    const reasons = new Set(vectors.phone.filter((v) => !v.valid).map((v) => v.reason));
    expect([...reasons].sort()).toEqual([...PHONE_REASONS].sort());
    expect(vectors.phone.filter((v) => !v.valid).every((v) => v.error === 'invalid_phone')).toBe(true);
  });

  it('keeps each valid phone consistent: digits are the normalized digits, 7 to 15 of them, or both null', () => {
    for (const v of vectors.phone.filter((c) => c.valid)) {
      if (v.normalized === null) {
        expect(v.digits, v.note).toBeNull();
        continue;
      }
      expect(v.digits, v.note).toBe(v.normalized.replace(/[^0-9]/g, ''));
      expect(v.digits.length, v.note).toBeGreaterThanOrEqual(7);
      expect(v.digits.length, v.note).toBeLessThanOrEqual(15);
    }
  });

  it('keeps each valid email consistent: one @, a lower-case domain, at most 254 characters', () => {
    for (const v of vectors.email.filter((c) => c.valid)) {
      const parts = v.normalized.split('@');
      expect(parts.length, v.note).toBe(2);
      expect(parts[1], v.note).toBe(parts[1].toLowerCase());
      expect(v.normalized.length, v.note).toBeLessThanOrEqual(254);
    }
  });

  it('defines trim_set as the Unicode White_Space set plus U+FEFF, as the platform knows it', () => {
    const platform = new Set();
    for (let c = 0; c <= 0xffff; c++) {
      if ((c >= 0xd800 && c <= 0xdfff) || !(/\s/u.test(String.fromCodePoint(c)) || c === 0x85)) continue;
      platform.add(c);
    }
    expect([...TRIM_SET].sort((a, b) => a - b)).toEqual([...platform].sort((a, b) => a - b));
    expect(TRIM_SET.size).toBe(26);
  });
});

describe('trimField', () => {
  it('strips exactly the trim_set code points from both ends, across the whole Basic Multilingual Plane', () => {
    const wrong = [];
    for (let c = 0; c <= 0xffff; c++) {
      if (c >= 0xd800 && c <= 0xdfff) continue;
      const ch = String.fromCodePoint(c);
      const input = `${ch}x${ch}`;
      const want = TRIM_SET.has(c) ? 'x' : input;
      if (trimField(input) !== want) wrong.push(`U+${c.toString(16).toUpperCase().padStart(4, '0')}`);
    }
    expect(wrong).toEqual([]);
  });

  it('leaves astral code points alone', () => {
    for (const cp of [0x1f600, 0xe0020, 0x10ffff]) {
      const ch = String.fromCodePoint(cp);
      expect(trimField(`${ch}x${ch}`)).toBe(`${ch}x${ch}`);
    }
  });

  it('strips a mixed run of whitespace at each end', () => {
    expect(trimField('\u00a0 \t\u3000\ufeffx\u0085\u2028\r\n ')).toBe('x');
  });

  it('never touches whitespace inside', () => {
    expect(trimField(' a \u00a0\t b ')).toBe('a \u00a0\t b');
  });

  it('turns whitespace alone into the empty string', () => {
    expect(trimField('\u00a0\u3000\ufeff\u0085')).toBe('');
    expect(trimField('')).toBe('');
  });
});

describe('validateEmail walks the shared vectors', () => {
  it.each(vectors.email)('$note', (v) => {
    expect(validateEmail(v.input)).toEqual(expectedEmail(v));
  });

  it('treats a missing email like null', () => {
    expect(validateEmail(undefined)).toEqual({ valid: false, error: 'invalid_email', reason: 'required' });
  });

  it.each([42, true, false, {}, [], ['buyer@example.com'], { email: 'buyer@example.com' }].map((v) => [v]))(
    'calls a non-string value malformed: %j',
    (value) => {
      expect(validateEmail(value)).toEqual({ valid: false, error: 'invalid_email', reason: 'malformed' });
    },
  );

  it('normalizes idempotently', () => {
    for (const v of vectors.email.filter((c) => c.valid)) {
      expect(validateEmail(v.normalized), v.note).toEqual({ valid: true, normalized: v.normalized });
    }
  });
});

describe('validatePhone walks the shared vectors', () => {
  it.each(vectors.phone)('$note', (v) => {
    expect(validatePhone(v.input)).toEqual(expectedPhone(v));
  });

  it('treats a missing phone like null: no phone given', () => {
    expect(validatePhone(undefined)).toEqual({ valid: true, normalized: null, digits: null });
  });

  it.each([5551234567, 0, true, false, {}, [], ['555-123-4567']].map((v) => [v]))('calls a non-string value malformed: %j', (value) => {
    expect(validatePhone(value)).toEqual({ valid: false, error: 'invalid_phone', reason: 'malformed' });
  });

  it('normalizes idempotently', () => {
    for (const v of vectors.phone.filter((c) => c.valid && c.normalized !== null)) {
      expect(validatePhone(v.normalized), v.note).toEqual({ valid: true, normalized: v.normalized, digits: v.digits });
    }
  });
});

describe('validateSource', () => {
  it.each([
    [null, null],
    [undefined, null],
    ['pricing', 'pricing'],
    ['/pricing#contact', '/pricing#contact'],
    ['landing.final-cta', 'landing.final-cta'],
    ['Pricing_Form', 'Pricing_Form'],
    ['a'.repeat(64), 'a'.repeat(64)],
  ])('accepts %j', (input, source) => {
    expect(validateSource(input)).toEqual({ valid: true, source });
  });

  it.each(['', ' pricing', 'pricing page', '<script>', 'a'.repeat(65), 'pr\u00efcing', 'pricing\n', 42, true, {}, []].map((v) => [v]))(
    'refuses %j',
    (input) => {
      expect(validateSource(input)).toEqual({ valid: false, error: 'invalid_source' });
    },
  );
});
