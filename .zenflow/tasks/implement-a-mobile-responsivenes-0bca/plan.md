# Full SDD workflow

## Configuration
- **Artifacts Path**: {@artifacts_path} → `.zenflow/tasks/{task_id}`

---

## Workflow Steps

### [x] Step: Requirements
<!-- chat-id: 0d316e23-946c-4644-81d7-7858d175bfb1 -->

Create a Product Requirements Document (PRD) based on the feature description.

1. Review existing codebase to understand current architecture and patterns
2. Analyze the feature definition and identify unclear aspects
3. Ask the user for clarifications on aspects that significantly impact scope or user experience
4. Make reasonable decisions for minor details based on context and conventions
5. If user can't clarify, make a decision, state the assumption, and continue

Save the PRD to `{@artifacts_path}/requirements.md`.

**Completed**: Created comprehensive `requirements.md` with:
- Problem analysis: Learn page tabs overflow on mobile (320px-768px viewports)
- 6 user stories covering mobile tab navigation, layout options, tablet support, consistent patterns, navigation, and readability
- 4 mobile tab layout design options (horizontal scroll, wrapped, accordion, bottom bar)
- Technical constraints for Dioxus framework and CSS architecture
- Two-phase approach: Phase 1 (Learn page fix) + Phase 2 (site-wide audit)
- Success metrics and acceptance criteria
- 4 open questions requiring user decisions (tab pattern, Phase 2 scope, testing strategy, collapsible content)

### [x] Step: Technical Specification
<!-- chat-id: 3f6c2318-a67b-4775-9eaa-efc6f79e791e -->

Create a technical specification based on the PRD in `{@artifacts_path}/requirements.md`.

1. Review existing codebase architecture and identify reusable components
2. Define the implementation approach

Save to `{@artifacts_path}/spec.md` with:
- Technical context (language, dependencies)
- Implementation approach referencing existing code patterns
- Source code structure changes
- Data model / API / interface changes
- Delivery phases (incremental, testable milestones)
- Verification approach using project lint/test commands

**Completed**: Created `spec.md` with:
- Accordion pattern design for Learn page mobile tabs
- 4-phase delivery plan (Learn page fix → Standardize breakpoints → Navigation → Polish)
- Component interface definitions
- CSS architecture additions
- Testing and verification approach

### [ ] Step: Planning

Create a detailed implementation plan based on `{@artifacts_path}/spec.md`.

1. Break down the work into concrete tasks
2. Each task should reference relevant contracts and include verification steps
3. Replace the Implementation step below with the planned tasks

Rule of thumb for step size: each step should represent a coherent unit of work (e.g., implement a component, add an API endpoint, write tests for a module). Avoid steps that are too granular (single function) or too broad (entire feature).

If the feature is trivial and doesn't warrant full specification, update this workflow to remove unnecessary steps and explain the reasoning to the user.

Save to `{@artifacts_path}/plan.md`.

### [ ] Step: Implementation

This step should be replaced with detailed implementation tasks from the Planning step.

If Planning didn't replace this step, execute the tasks in `{@artifacts_path}/plan.md`, updating checkboxes as you go. Run planned tests/lint and record results in plan.md.
