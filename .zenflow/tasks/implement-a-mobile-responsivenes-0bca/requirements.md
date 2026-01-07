# Product Requirements Document (PRD)
## Mobile Responsiveness for Logos Website

**Version**: 1.0
**Date**: 2026-01-07
**Status**: Draft - Awaiting Approval

---

## 1. Executive Summary

### Problem Statement
Customer feedback indicates the Logos website is performing poorly on mobile devices. Specifically:
- Learn page tab buttons (Practice, Test) run off the page on mobile viewports
- Users cannot see or interact with all tab options on smaller screens
- Overall site lacks consistent mobile-first design patterns

### Proposed Solution
Implement comprehensive mobile responsiveness across the entire website with two phases:
1. **Phase 1 (Critical)**: Fix Learn page tab navigation to be fully functional on mobile devices
2. **Phase 2 (Enhancement)**: Apply best-practice responsive design patterns site-wide

### Success Criteria
- All Learn page tabs are visible and usable on mobile devices (320px - 768px width)
- Touch targets meet WCAG 2.5 standards (minimum 44x44px)
- Site follows modern mobile-first design patterns
- Code demonstrates reusable, maintainable responsive patterns for other developers

---

## 2. Background & Context

### Current Architecture
- **Framework**: Dioxus (Rust WebAssembly framework)
- **Styling**: Inline CSS strings embedded in Rust files
- **Existing Mobile Support**: `src/ui/responsive.rs` contains comprehensive mobile utilities including:
  - Breakpoint definitions (XS: 375px, SM: 640px, MD: 768px, LG: 1024px, XL: 1280px)
  - Pre-built mobile tab bar styles (`.mobile-tabs`, `.mobile-tab`)
  - Touch target standards (44px minimum)
  - Safe area inset support for notched devices

### Current State of Learn Page
**Location**: `src/ui/pages/learn.rs`

**Tab Implementation** (Lines 1599-1631):
- 4 tabs: Lesson, Examples, Practice, Test
- CSS classes: `.content-tabs` (container), `.content-tab-btn` (buttons)
- Special styling: `.practice` (green accent), `.test` (orange/yellow accent)
- Current breakpoints: `@media (max-width: 1024px)` and `@media (max-width: 640px)` exist elsewhere in the file, but NOT for tabs

**Identified Issues**:
1. `.content-tabs` and `.content-tab-btn` have NO mobile media queries
2. Fixed padding (`8px 16px`) doesn't scale down for narrow viewports
3. No horizontal scroll or wrapping behavior for overflow
4. Missing touch target minimum height/width
5. Existing `.mobile-tabs` utility styles in `responsive.rs` are NOT being used

### Pages with Media Queries (Partial Mobile Support)
Based on grep results, these pages have some responsive styling:
- `landing.rs`, `learn.rs`, `pricing.rs`, `roadmap.rs`, `studio.rs`, `workspace.rs`
- Components: `main_nav.rs`, `editor.rs`, `learn_sidebar.rs`, `guide_sidebar.rs`, `ast_tree.rs`, `logic_output.rs`

### Pages Requiring Review
All pages in `src/ui/pages/`:
- landing, learn, lesson, pricing, privacy, profile, review, roadmap, studio, success, terms, workspace

---

## 3. User Stories & Requirements

### 3.1 Critical Requirements (Phase 1: Learn Page)

#### US-1: Mobile Tab Navigation
**As a** mobile user learning Logos
**I want to** access all tab options (Lesson, Examples, Practice, Test)
**So that** I can switch between content types without horizontal scrolling

**Acceptance Criteria**:
- [ ] All 4 tabs visible on iPhone SE (375px width)
- [ ] All 4 tabs visible on standard mobile (320px - 428px width)
- [ ] Touch targets meet 44x44px minimum (WCAG 2.5)
- [ ] Active tab state clearly visible
- [ ] Color-coded styling preserved (green Practice, yellow Test)
- [ ] Smooth transitions between tab states

#### US-2: Tab Layout Options
**As a** product owner
**I want to** choose the most intuitive mobile tab layout
**So that** users have the best experience on small screens

**Design Options** (User Decision Required):

**Option A: Horizontal Scroll Tabs**
- Tabs remain horizontal, scrollable with touch
- Uses existing `.mobile-tabs` utility from `responsive.rs`
- Pros: Familiar pattern, minimal layout change
- Cons: Requires scroll gesture, less discoverable

**Option B: Stacked/Wrapped Tabs**
- Tabs wrap to 2 rows on mobile (2x2 grid)
- Pros: All tabs visible without scrolling
- Cons: Takes more vertical space

**Option C: Collapsible Accordion**
- Tabs become expandable sections (as mentioned in task description)
- Click tab name to expand content inline
- Pros: Novel, space-efficient
- Cons: More complex interaction, different pattern from desktop

**Option D: Bottom Tab Bar**
- Fixed bottom navigation bar (native app pattern)
- Pros: Thumb-friendly, modern mobile pattern
- Cons: Takes persistent screen space

**Recommendation**: Option A (Horizontal Scroll) for initial implementation
- Leverages existing `.mobile-tabs` utility
- Fastest to implement
- Proven mobile pattern
- Can iterate to Option B/C/D based on user feedback

#### US-3: Tablet Support
**As a** tablet user (768px - 1024px)
**I want to** see an optimized layout between mobile and desktop
**So that** I get the best experience for my screen size

**Acceptance Criteria**:
- [ ] Tabs use appropriate sizing at 768px - 1024px breakpoint
- [ ] Touch targets remain 44x44px minimum
- [ ] Layout transitions smoothly between breakpoints

### 3.2 Enhancement Requirements (Phase 2: Site-Wide)

#### US-4: Consistent Mobile Patterns
**As a** developer maintaining the codebase
**I want** consistent responsive patterns across all pages
**So that** the code is maintainable and extensible

**Acceptance Criteria**:
- [ ] Document mobile-first design patterns in code comments
- [ ] Create reusable responsive utilities/components
- [ ] Apply patterns consistently across all pages

#### US-5: Mobile Navigation
**As a** mobile user
**I want** easy navigation across all pages
**So that** I can access all site features on my device

**Acceptance Criteria**:
- [ ] Main navigation works on mobile (hamburger menu if needed)
- [ ] All interactive elements have sufficient touch targets
- [ ] No horizontal overflow on any page

#### US-6: Content Readability
**As a** mobile user
**I want** readable text and properly sized elements
**So that** I can consume content without zooming

**Acceptance Criteria**:
- [ ] Font sizes scale appropriately for mobile (14px+ body text)
- [ ] Line heights optimized for readability (1.5 - 1.75)
- [ ] Padding/margins use mobile-appropriate spacing
- [ ] Code blocks scrollable on mobile

---

## 4. Design Requirements

### 4.1 Breakpoints (from `responsive.rs`)
```rust
XS: 375px   // Small phones (iPhone SE)
SM: 640px   // Standard phones
MD: 768px   // Tablets (portrait)
LG: 1024px  // Tablets (landscape) / Small laptops
XL: 1280px  // Desktop
```

**Primary Mobile Breakpoint**: `@media (max-width: 768px)`

### 4.2 Touch Targets
- Minimum: 44x44px (WCAG 2.5.5 Level AAA)
- Recommended: 48x48px for primary actions
- Spacing between targets: minimum 8px

### 4.3 Typography (Mobile)
- Body text: minimum 14px (16px recommended)
- Headings: scale appropriately (use `clamp()` for fluid typography)
- Line height: 1.5 - 1.75 for body text

### 4.4 Spacing (Mobile)
- Container padding: 12px - 16px (use `var(--mobile-padding, 12px)`)
- Element gaps: 8px - 12px
- Section spacing: 24px - 32px

### 4.5 Visual Design
- Preserve existing color scheme (dark theme)
- Maintain brand colors:
  - Practice: `var(--color-success)` (green #4ade80)
  - Test: `#fbbf24` (yellow/orange)
  - Accent: `var(--color-accent-blue)` (#667eea)
- Maintain existing transition timings (0.18s - 0.2s)

---

## 5. Technical Constraints

### 5.1 Framework Limitations
- **Dioxus RSX**: Styles must be embedded as Rust string constants
- **No CSS-in-JS**: Cannot use dynamic style generation
- **No Tailwind**: Must use custom CSS with CSS variables

### 5.2 Browser Support
- Modern mobile browsers (iOS Safari 14+, Chrome Mobile 90+)
- Support for notched devices (safe-area-inset)
- Touch gesture support (-webkit-overflow-scrolling)

### 5.3 Performance
- Minimize CSS payload (reuse existing utilities)
- Avoid layout thrashing (use CSS transforms for animations)
- Optimize for 3G/4G networks (minimal asset loading)

### 5.4 Accessibility
- WCAG 2.1 Level AA minimum (AAA for touch targets)
- Keyboard navigation support
- Screen reader compatibility
- Reduced motion support (`prefers-reduced-motion`)

---

## 6. Out of Scope

### Explicitly NOT Included
1. **Desktop layout changes**: Only mobile-specific modifications
2. **New features**: No new tabs, content, or functionality
3. **Content restructuring**: Preserve existing information architecture
4. **Backend changes**: Pure frontend/CSS implementation
5. **Dark/Light theme toggle**: Maintain existing dark theme only
6. **Internationalization**: English-only for now
7. **Browser compatibility for IE/legacy**: Modern browsers only

---

## 7. Dependencies & Assumptions

### Dependencies
- Existing `src/ui/responsive.rs` utilities
- Existing CSS variables in `src/ui/theme.rs`
- Dioxus framework (current version)

### Assumptions
1. Users access site primarily on modern mobile browsers
2. Existing desktop layout is satisfactory and should be preserved
3. Learn page is highest priority for mobile users
4. Horizontal scroll is acceptable for tabs (if Option A chosen)
5. No breaking changes to desktop experience

### Questions for User (Clarification Needed)

**Q1: Tab Layout Pattern**
Which mobile tab layout pattern do you prefer for the Learn page?
- **Option A**: Horizontal scroll tabs (recommended, fastest)
- **Option B**: Wrapped/stacked tabs (2x2 grid)
- **Option C**: Collapsible accordion sections
- **Option D**: Bottom tab bar (native app style)

**Q2: Phase 2 Priority**
After fixing Learn page, should we:
- **Option A**: Systematically audit and fix ALL pages (comprehensive, slower)
- **Option B**: Fix only pages with reported issues (targeted, faster)
- **Option C**: Create pattern library and document, fix pages incrementally

**Q3: Testing Scope**
Should we write:
- **Option A**: Visual regression tests (screenshot comparison)
- **Option B**: Unit tests for responsive utilities only
- **Option C**: Manual testing checklist only (fastest)
- **Option D**: E2E tests for critical mobile flows

**Q4: Collapsible Content**
The task description mentions "collapsable section to learn, practice, and quizes." Should the content within each tab ALSO be collapsible (accordions within tabs), or just the tab navigation itself?

---

## 8. Success Metrics

### Quantitative Metrics
- [ ] All pages render without horizontal overflow on 320px - 428px width
- [ ] Touch targets meet 44x44px minimum on all interactive elements
- [ ] Page load time remains under 3s on 4G
- [ ] Lighthouse mobile score improves (target: 90+)

### Qualitative Metrics
- [ ] User feedback indicates mobile experience is improved
- [ ] Developers can easily apply responsive patterns to new pages
- [ ] Code review confirms patterns follow best practices

### Testing Checklist (Mobile Devices)
- [ ] iPhone SE (375x667) - smallest modern iPhone
- [ ] iPhone 12/13/14 (390x844) - standard size
- [ ] iPhone 14 Pro Max (428x926) - largest iPhone
- [ ] Samsung Galaxy S20 (360x800) - standard Android
- [ ] iPad (768x1024) - tablet portrait
- [ ] iPad Pro (1024x1366) - tablet landscape

---

## 9. Implementation Phases

### Phase 1: Learn Page Tab Fix (Critical)
**Estimated Effort**: 1-2 days
**Priority**: P0 - Blocking customer issues

**Deliverables**:
1. Mobile-responsive tab navigation on Learn page
2. Touch-friendly tab buttons
3. Preserved visual styling (colors, transitions)
4. Tablet-optimized layout
5. Tests for tab functionality

### Phase 2: Site-Wide Responsive Audit (Enhancement)
**Estimated Effort**: 3-5 days
**Priority**: P1 - High priority improvements

**Deliverables**:
1. Audit all pages for mobile issues
2. Apply consistent responsive patterns
3. Document best practices for future development
4. Create reusable responsive utilities
5. Update components (nav, sidebar, editor, etc.)
6. Comprehensive mobile testing

---

## 10. Acceptance Criteria Summary

### Phase 1 - Learn Page (Must Have)
- [ ] All 4 tabs visible and clickable on mobile (320px - 768px)
- [ ] Touch targets minimum 44x44px
- [ ] No horizontal overflow on Learn page
- [ ] Active/inactive states clearly visible
- [ ] Color coding preserved (green Practice, yellow Test)
- [ ] Smooth on tablets (768px - 1024px)
- [ ] Works on iOS Safari and Chrome Mobile
- [ ] No regression on desktop layout

### Phase 2 - Site-Wide (Should Have)
- [ ] All pages responsive on mobile
- [ ] Main navigation functional on mobile
- [ ] All interactive elements meet touch target requirements
- [ ] No horizontal overflow on any page
- [ ] Consistent spacing and typography across pages
- [ ] Code comments documenting responsive patterns
- [ ] Reusable utility styles for common patterns

---

## 11. Open Questions & Decisions

### Decisions Required
1. ⏳ **Tab layout pattern** (Options A/B/C/D) - Awaiting user input
2. ⏳ **Phase 2 approach** (All pages vs targeted vs incremental) - Awaiting user input
3. ⏳ **Testing strategy** (Visual/Unit/Manual/E2E) - Awaiting user input
4. ⏳ **Collapsible content scope** (Tabs only vs content within tabs) - Awaiting user input

### Assumptions Made (User can override)
1. ✅ Horizontal scroll tabs (Option A) is acceptable initial approach
2. ✅ Desktop layout should remain unchanged
3. ✅ Dark theme only (no light theme needed)
4. ✅ Modern browsers only (no IE support)
5. ✅ Manual testing is sufficient for initial implementation

---

## 12. References

### Existing Code
- Tab implementation: `src/ui/pages/learn.rs:1599-1631`
- Tab styles: `src/ui/pages/learn.rs:419-481`
- Mobile utilities: `src/ui/responsive.rs`
- Theme variables: `src/ui/theme.rs`
- Global styles: `src/ui/app.rs`

### Design Resources
- WCAG 2.5.5 Touch Target Size: https://www.w3.org/WAI/WCAG21/Understanding/target-size.html
- Mobile-First CSS: https://web.dev/responsive-web-design-basics/
- Touch Gestures: https://developer.mozilla.org/en-US/docs/Web/API/Touch_events

### Similar Patterns (Internal)
- Mobile tab bar: `responsive.rs:161-272` (`.mobile-tabs` utility)
- Mobile panels: `responsive.rs:279+` (`.mobile-panel` utility)
- Mobile buttons: `responsive.rs` (`.mobile-btn` utility)

---

## Appendix A: Current Learn Page Tab CSS

```css
/* Current implementation (NO mobile support) */
.content-tabs {
    display: flex;
    gap: var(--spacing-sm);  /* 8px */
    margin-bottom: var(--spacing-xl);
    border-bottom: 1px solid rgba(255,255,255,0.08);
    padding-bottom: var(--spacing-md);
}

.content-tab-btn {
    padding: 8px 16px;
    border-radius: var(--radius-md) var(--radius-md) 0 0;
    font-size: var(--font-body-sm);  /* 15px */
    font-weight: 600;
    cursor: pointer;
    transition: all 0.18s ease;
    border: none;
    background: transparent;
    color: var(--text-tertiary);
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
}
```

**Problem**: No `@media` queries for mobile viewports, causing overflow on screens < 400px.

---

## Appendix B: Available Mobile Utilities

From `src/ui/responsive.rs`:

```rust
// Already implemented and ready to use:
MOBILE_TAB_BAR_STYLES    // Lines 161-272
MOBILE_PANEL_STYLES      // Lines 279+
MOBILE_BUTTON_STYLES     // (exists in file)
MEDIA_MOBILE             // "@media (max-width: 768px)"
MEDIA_TABLET             // "@media (min-width: 769px) and (max-width: 1024px)"
TOUCH_MIN                // "44px" - WCAG minimum
```

These utilities are production-ready and can be integrated into Learn page.

---

**END OF PRD**
