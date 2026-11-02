//! The contact form's rules: what a valid email and phone are, the request the form sends, and
//! what the worker's answer means.
//!
//! The worker behind [`ENDPOINT`] applies the same rules (`services/leads/src/lead.js`), and both
//! walk one set of vectors (`crates/logicaffeine_tests/tests/fixtures/lead_vectors.json`), so the
//! page never refuses what the worker would take, or sends what it would refuse.

use serde_json::{json, Value};

/// Where the form sends its request.
pub const ENDPOINT: &str = "https://api.logicaffeine.com/lead";

/// The form's tag in every request, so the team can tell where a request came from.
/// The page's tag on the wire. It keeps its old name: the lead worker is deployed on it, and a lead
/// from the contact page is the same lead.
pub const SOURCE: &str = "pricing";

/// The action the form's Turnstile widget stamps on its token; the worker's siteverify check
/// refuses a token that carries any other.
pub const TURNSTILE_ACTION: &str = "pricing-lead";

const MAX_EMAIL: usize = 254;
const MAX_LOCAL: usize = 64;
const MAX_LABEL: usize = 63;
const MAX_PHONE: usize = 32;
const MIN_DIGITS: usize = 7;
const MAX_DIGITS: usize = 15;

/// `value` without leading or trailing Unicode white space or byte-order marks. `str::trim`
/// keeps U+FEFF, which a paste from some apps carries, so it is not the rule.
pub fn trim_field(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || c == '\u{FEFF}')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailError {
    Required,
    Malformed,
}

impl EmailError {
    /// Every error the email field can show.
    pub const ALL: [EmailError; 2] = [EmailError::Required, EmailError::Malformed];

    /// The worker's name for it.
    pub fn reason(self) -> &'static str {
        match self {
            EmailError::Required => "required",
            EmailError::Malformed => "malformed",
        }
    }

    fn from_reason(reason: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.reason() == reason)
    }

    /// What the field says under it.
    pub fn message(self) -> &'static str {
        match self {
            EmailError::Required => "Enter your work email, so the team can reply.",
            EmailError::Malformed => "Enter a whole email address, with an @ and a domain.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhoneError {
    BadCharacter,
    MisplacedPlus,
    TooLong,
    TooFewDigits,
    TooManyDigits,
}

impl PhoneError {
    /// Every error the phone field can show.
    pub const ALL: [PhoneError; 5] =
        [PhoneError::BadCharacter, PhoneError::MisplacedPlus, PhoneError::TooLong, PhoneError::TooFewDigits, PhoneError::TooManyDigits];

    /// The worker's name for it.
    pub fn reason(self) -> &'static str {
        match self {
            PhoneError::BadCharacter => "bad_character",
            PhoneError::MisplacedPlus => "misplaced_plus",
            PhoneError::TooLong => "too_long",
            PhoneError::TooFewDigits => "too_few_digits",
            PhoneError::TooManyDigits => "too_many_digits",
        }
    }

    fn from_reason(reason: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.reason() == reason)
    }

    /// What the field says under it.
    pub fn message(self) -> &'static str {
        match self {
            PhoneError::BadCharacter => "Use only digits, spaces, a plus sign, parentheses, hyphens and periods.",
            PhoneError::MisplacedPlus => "Put the + only at the start of the number.",
            PhoneError::TooLong => "Keep the phone number to 32 characters or fewer.",
            PhoneError::TooFewDigits => "Enter the whole number: it needs at least 7 digits.",
            PhoneError::TooManyDigits => "A phone number has at most 15 digits.",
        }
    }
}

/// A valid email: the trimmed local part as typed, `@`, the domain lower-cased.
pub fn validate_email(input: Option<&str>) -> Result<String, EmailError> {
    let email = trim_field(input.ok_or(EmailError::Required)?);
    if email.is_empty() {
        return Err(EmailError::Required);
    }
    if email.len() > MAX_EMAIL || !email.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(EmailError::Malformed);
    }
    let (local, domain) = email.split_once('@').ok_or(EmailError::Malformed)?;
    if domain.contains('@') {
        return Err(EmailError::Malformed);
    }
    let atom = |a: &str| !a.is_empty() && a.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+/=?^_`{|}~-".contains(&b));
    if local.len() > MAX_LOCAL || !local.split('.').all(atom) {
        return Err(EmailError::Malformed);
    }
    let label = |l: &str| {
        let bytes = l.as_bytes();
        (1..=MAX_LABEL).contains(&bytes.len())
            && bytes.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-')
            && bytes[0] != b'-'
            && bytes[bytes.len() - 1] != b'-'
    };
    let labels: Vec<&str> = domain.split('.').collect();
    let top = labels[labels.len() - 1];
    if labels.len() < 2 || !labels.iter().all(|l| label(l)) || top.len() < 2 || top.bytes().all(|b| b.is_ascii_digit()) {
        return Err(EmailError::Malformed);
    }
    Ok(format!("{local}@{}", domain.to_ascii_lowercase()))
}

/// A phone as the worker stores it: the trimmed number as typed, and its digits alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phone {
    pub normalized: String,
    pub digits: String,
}

/// A phone is optional: nothing, or only white space, is no phone.
pub fn validate_phone(input: Option<&str>) -> Result<Option<Phone>, PhoneError> {
    let Some(input) = input else { return Ok(None) };
    let phone = trim_field(input);
    if phone.is_empty() {
        return Ok(None);
    }
    if !phone.chars().all(|c| c.is_ascii_digit() || " +()-.".contains(c)) {
        return Err(PhoneError::BadCharacter);
    }
    if phone.rfind('+').is_some_and(|at| at > 0) {
        return Err(PhoneError::MisplacedPlus);
    }
    if phone.len() > MAX_PHONE {
        return Err(PhoneError::TooLong);
    }
    let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < MIN_DIGITS {
        return Err(PhoneError::TooFewDigits);
    }
    if digits.len() > MAX_DIGITS {
        return Err(PhoneError::TooManyDigits);
    }
    Ok(Some(Phone { normalized: phone.to_string(), digits }))
}

/// What is wrong with each field; nothing when both are fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FieldErrors {
    pub email: Option<EmailError>,
    pub phone: Option<PhoneError>,
}

impl FieldErrors {
    pub fn is_empty(&self) -> bool {
        self.email.is_none() && self.phone.is_none()
    }
}

/// The JSON the worker reads, `{ email, phone, turnstile_token, source, website }`, or what is
/// wrong with the fields. The honeypot (`website`) rides untouched: the worker decides what a
/// filled one means.
pub fn request_body(email: &str, phone: &str, turnstile_token: &str, website: &str) -> Result<Value, FieldErrors> {
    match (validate_email(Some(email)), validate_phone(Some(phone))) {
        (Ok(email), Ok(phone)) => Ok(json!({
            "email": email,
            "phone": phone.map(|p| p.normalized),
            "turnstile_token": turnstile_token,
            "source": SOURCE,
            "website": website,
        })),
        (email, phone) => Err(FieldErrors { email: email.err(), phone: phone.err() }),
    }
}

/// What the worker's answer means for the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Stored: the team has it.
    Sent,
    /// The worker refused a field: the send failed, and the form also says why under that field.
    Invalid(FieldErrors),
    /// Any other answer: the form offers the team's address instead.
    Failed,
}

impl Outcome {
    pub fn of(status: u16, body: &str) -> Outcome {
        let answer: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        match status {
            202 if answer["ok"] == true => Outcome::Sent,
            400 => {
                let reason = answer["reason"].as_str().unwrap_or("");
                let errors = match answer["error"].as_str() {
                    Some("invalid_email") => EmailError::from_reason(reason).map(|e| FieldErrors { email: Some(e), phone: None }),
                    Some("invalid_phone") => PhoneError::from_reason(reason).map(|e| FieldErrors { email: None, phone: Some(e) }),
                    _ => None,
                };
                errors.map_or(Outcome::Failed, Outcome::Invalid)
            }
            _ => Outcome::Failed,
        }
    }
}

#[cfg(test)]
#[path = "../../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/contact/lead/tests.rs"]
mod tests;
