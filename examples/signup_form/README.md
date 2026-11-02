# Signup form — validation, an error summary, disabled while invalid

`forms.lg` validation, an error summary listing every problem, `.prevent` so
a handled submit never triggers a native navigation, and a submit button
disabled until the form is valid.

```bash
largo test           # runs the ## Test blocks against the tree-walker and the VM
largo serve          # http://localhost:8080/signup
largo build --static # dist/ - upload it anywhere
```

## What it shows

**Two independent notions of "valid".** `errors` is a plain `State:` field,
written only inside `To submit:` — nothing is scolded before a first attempt.
`formValid` is a *derived* `Bool`, recomputed on every keystroke from the live
field values, and is the only thing `disabled={not formValid}` reads. The tests
assert both states of the button directly: `Expect that "#submit-btn" is disabled.`
while any field is invalid, and `Expect that "#submit-btn" is not disabled.` the
moment all three are valid.

**An error summary built from the list, not just `errorSummary`.**
`errorFor(errors, field)` gives each field's own message. The summary at the
top loops over the `Seq of FieldError` itself (`{for each e in errors, keyed
by e's field:}`), so it renders as a real `<ul>` a test can count
(`.summary-item`) beside the per-field messages `forms.lg` derives from the
same `errors` value.

**The page's validation, not the browser's.** The form carries `novalidate`.
Without it a browser runs its own constraint validation before `submit` — the
`type="email"` field would stop a submission of `ada` with the browser's
bubble, and `To submit:` would never run to show this page's messages.
Every execution tier models that validation (`Submit` is `requestSubmit`), so
`novalidate` is what makes the page's own summary the one a visitor sees.

**Order general to particular.** `checkRequired` then `checkEmail` on the
same field: the first to fail is the one kept, so an empty email says
"required," never "does not look like an email address."
