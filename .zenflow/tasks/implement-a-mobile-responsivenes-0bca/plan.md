# Mobile Responsiveness Implementation Plan

## Configuration
- **Artifacts Path**: `.zenflow/tasks/implement-a-mobile-responsivenes-0bca`
- **Requirements**: `requirements.md`
- **Technical Spec**: `spec.md`

---

## Workflow Steps

### [x] Step: Requirements
<!-- chat-id: 0d316e23-946c-4644-81d7-7858d175bfb1 -->
**Completed**: Created comprehensive `requirements.md` covering problem analysis, user stories, design options, and acceptance criteria.

### [x] Step: Technical Specification
<!-- chat-id: 3f6c2318-a67b-4775-9eaa-efc6f79e791e -->
**Completed**: Created `spec.md` with accordion pattern design for Learn page mobile tabs.

### [x] Step: Planning
<!-- chat-id: b5faf5fa-0a12-434c-b026-5a05c71ed0c0 -->
**Completed**: Created detailed implementation tasks below.

---

## Phase 1: Learn Page Mobile Fix (Critical)

### [ ] Task 1.1: Add Accordion Styles to Responsive Module
**File**: `src/ui/responsive.rs`

Add mobile accordion tab styles as a new constant `MOBILE_ACCORDION_STYLES`:
- `.accordion-tabs`: Full-width flex column container, hidden on desktop
- `.accordion-tab-header`: 48px min-height touch target, flex row with expand/collapse icon
- `.accordion-tab-content`: Animated max-height transition for expand/collapse
- `.accordion-tab-content.expanded`: Visible state
- Color variants: `.accordion-tab-header.lesson`, `.practice`, `.examples`, `.test`
- Desktop media query to hide accordion and show standard tabs

**Verification**:
```bash
cargo build --features cli
```

### [ ] Task 1.2: Create MobileAccordionTabs Component
**File**: `src/ui/components/module_tabs.rs`

Extend or create a mobile-specific variant:
1. Add `MobileAccordionTabs` component that renders stacked accordion headers
2. Props: `current: TabMode`, `on_change: EventHandler<TabMode>`, `locked_tabs: Vec<TabMode>`
3. Each tab header shows: icon, label, expand/collapse chevron indicator
4. Only one tab expanded at a time (controlled by `current` prop)
5. Locked tabs show lock icon and disabled styling

**Verification**:
```bash
cargo build --features cli
cargo test -- --skip e2e
```

### [ ] Task 1.3: Integrate Accordion into Learn Page
**File**: `src/ui/pages/learn.rs`

1. Import `MOBILE_ACCORDION_STYLES` from responsive module
2. Add accordion markup in the tab section that shows on mobile only
3. Keep existing horizontal tabs for desktop (shown with `.desktop-only`)
4. Wire accordion tab changes to the existing `set_tab_mode` signal
5. Ensure content sections properly show/hide based on selected tab

**Verification**:
- Build: `cargo build --features cli`
- Manual test: Open in browser, resize to mobile viewport, verify tabs stack and expand
- Verify no horizontal scroll at 320px width

### [ ] Task 1.4: Test Learn Page Mobile Tabs
**File**: `tests/` (if UI tests exist) or manual testing

Verification checklist:
- [ ] All 4 tabs visible and tappable on 320px viewport
- [ ] Touch targets meet 44px minimum height
- [ ] Active tab content displays correctly
- [ ] Tab switch animation is smooth
- [ ] Color coding preserved (green Practice, yellow Test)
- [ ] Desktop layout unchanged (horizontal tabs)
- [ ] Tablet layout works (768px breakpoint)

```bash
cargo test -- --skip e2e
```

---

## Phase 2: Standardize Breakpoints Site-Wide

### [ ] Task 2.1: Standardize Landing Page Breakpoints
**File**: `src/ui/pages/landing.rs`

1. Replace custom breakpoints (700px, 980px) with standard ones from `responsive.rs` (480px, 768px, 1024px)
2. Add XS breakpoint (480px) handling for small phones
3. Ensure hero section, feature grids, and CTAs work at 320px width

**Verification**:
```bash
cargo build --features cli
```
Manual: Check landing page at 320px, 480px, 768px, 1024px viewports

### [ ] Task 2.2: Standardize Pricing Page Breakpoints
**File**: `src/ui/pages/pricing.rs`

1. Replace any custom breakpoints with standard ones
2. Ensure pricing cards stack vertically on mobile
3. Add XS breakpoint for small phones
4. Verify touch targets on pricing buttons

**Verification**:
```bash
cargo build --features cli
```

### [ ] Task 2.3: Add Mobile Breakpoints to Lesson Page
**File**: `src/ui/pages/lesson.rs`

1. Add media queries for MD (768px) and XS (480px) breakpoints
2. Ensure lesson content is readable on mobile
3. Stack any multi-column layouts on small screens

**Verification**:
```bash
cargo build --features cli
```

### [ ] Task 2.4: Add Mobile Breakpoints to Profile Page
**File**: `src/ui/pages/profile.rs`

1. Add responsive breakpoints if missing
2. Ensure form inputs and buttons meet touch target requirements
3. Stack layouts appropriately on mobile

**Verification**:
```bash
cargo build --features cli
```

### [ ] Task 2.5: Standardize Workspace Page Breakpoints
**File**: `src/ui/pages/workspace.rs`

1. Add tablet breakpoints (768px)
2. Add phone breakpoints (480px)
3. Ensure panels/sections stack correctly on mobile

**Verification**:
```bash
cargo build --features cli
```

---

## Phase 3: Navigation Enhancement

### [ ] Task 3.1: Create Hamburger Menu Component
**File**: `src/ui/components/hamburger_menu.rs` (new file)

1. Create `HamburgerMenu` component with open/close state
2. Three-line icon that animates to X when open
3. Props: `is_open: Signal<bool>`, `on_toggle: EventHandler<()>`
4. Mobile-only visibility (hidden on desktop)

**Verification**:
```bash
cargo build --features cli
```

### [ ] Task 3.2: Create Mobile Navigation Drawer
**File**: `src/ui/components/nav_drawer.rs` (new file)

1. Slide-out navigation drawer component
2. Full-height overlay from left side
3. All main navigation links
4. Close button and click-outside-to-close
5. Smooth slide animation

**Verification**:
```bash
cargo build --features cli
```

### [ ] Task 3.3: Integrate Mobile Navigation into MainNav
**File**: `src/ui/components/main_nav.rs`

1. Import hamburger menu and nav drawer components
2. Show hamburger icon at breakpoint <640px instead of hiding nav links
3. Wire hamburger to open/close the nav drawer
4. Preserve desktop navigation unchanged

**Verification**:
```bash
cargo build --features cli
cargo test -- --skip e2e
```
Manual: Verify mobile nav works at 320px-639px viewports

---

## Phase 4: Polish & Optimization

### [ ] Task 4.1: Audit All Touch Targets Site-Wide
**Files**: All pages in `src/ui/pages/`

1. Check all buttons, links, and interactive elements
2. Ensure minimum 44x44px touch target on mobile
3. Add padding or min-height/width where needed
4. Document any elements that cannot meet target (with justification)

**Verification**:
Manual testing on each page at mobile viewport

### [ ] Task 4.2: Add Safe Area Inset Support
**Files**: `src/ui/pages/learn.rs`, `main_nav.rs`, other pages with fixed elements

1. Apply `env(safe-area-inset-*)` to fixed/sticky elements
2. Ensure content isn't hidden behind notches or home indicators
3. Test with iOS device emulation in browser dev tools

**Verification**:
Manual: Use Safari responsive mode with iPhone X/14 Pro simulation

### [ ] Task 4.3: Add Reduced Motion Support
**Files**: `src/ui/responsive.rs`, affected components

1. Wrap animations in `@media (prefers-reduced-motion: reduce)` checks
2. Provide static alternatives for users who prefer reduced motion
3. Test with browser reduced motion setting enabled

**Verification**:
```bash
cargo build --features cli
```
Manual: Enable "reduce motion" in OS settings, verify no jarring animations

### [ ] Task 4.4: Final Mobile Testing & Documentation
**Files**: Various

1. Test all pages on device matrix:
   - iPhone SE (320px)
   - iPhone 14 (393px)
   - iPhone 14 Pro Max (428px)
   - Samsung Galaxy S21 (360px)
   - iPad Mini (768px)
2. Document any known limitations
3. Add inline code comments explaining mobile patterns used
4. Update any existing documentation

**Verification**:
```bash
cargo test -- --skip e2e
cargo build --features cli
```
Full manual testing checklist complete

---

## Success Criteria

### Phase 1 (Critical - Must pass before merging)
- [ ] All 4 Learn page tabs visible and usable on 320px viewport
- [ ] Touch targets meet 44px minimum
- [ ] No horizontal scroll on Learn page at mobile widths
- [ ] Desktop layout unchanged
- [ ] `cargo test -- --skip e2e` passes
- [ ] `cargo build --features cli` succeeds

### Phase 2 (Enhancement)
- [ ] All pages use standard breakpoints from `responsive.rs`
- [ ] No horizontal overflow on any page at 320px
- [ ] Consistent spacing and typography across pages

### Phase 3 (Enhancement)
- [ ] Mobile navigation accessible via hamburger menu
- [ ] Navigation drawer opens/closes smoothly
- [ ] All nav links accessible on mobile

### Phase 4 (Polish)
- [ ] All interactive elements meet touch target requirements
- [ ] Safe area support for notched devices
- [ ] Reduced motion preference respected
- [ ] Manual testing on device matrix complete
