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

### [x] Task 1.1: Add Accordion Styles to Responsive Module
<!-- chat-id: c1f342a2-9468-488f-a170-d91c68441e11 -->
**File**: `src/ui/responsive.rs`

**Completed**: Added `MOBILE_ACCORDION_STYLES` constant with:
- `.accordion-tabs`: Full-width flex column container, hidden on desktop, shown on mobile
- `.accordion-tab-item`: Individual accordion item with border and rounded corners
- `.accordion-tab-header`: 48px min-height touch target with flex layout and chevron indicator
- `.accordion-tab-content`: Animated max-height transition (0.25s collapse, 0.35s expand)
- `.accordion-tab-content.expanded`: Visible state with 2000px max-height
- Color variants for lesson (blue), examples (purple), practice (green), test (yellow)
- Locked tab styling with opacity and cursor changes
- `@media (prefers-reduced-motion: reduce)` support
- Updated `all_mobile_styles()` to include accordion styles

**Verification**: `cargo build --features cli` ✓

### [x] Task 1.2: Create MobileAccordionTabs Component
<!-- chat-id: b37cb273-a9b4-4046-8ee6-e4a122730f9b -->
**File**: `src/ui/components/module_tabs.rs`

**Completed**: Added two mobile accordion components:

1. **`MobileAccordionTabs`**: Container component for all four tabs
   - Props: `current: TabMode`, `on_change: EventHandler<TabMode>`, `locked_tabs: Vec<TabMode>`, `children: Element`
   - Renders stacked accordion headers with icons and chevron indicators
   - Only one tab expanded at a time (controlled by `current` prop)
   - Uses `MOBILE_ACCORDION_STYLES` from responsive module

2. **`MobileAccordionTab`**: Individual accordion tab for custom layouts
   - Props: `tab: TabMode`, `is_expanded: bool`, `is_locked: bool`, `on_click: EventHandler<TabMode>`, `children: Element`
   - Provides granular control when tab content is generated dynamically

3. **Helper functions**:
   - `tab_icon(TabMode)`: Returns emoji icon for each tab (📖, 💡, ✏️, 📝)
   - `tab_class_name(TabMode)`: Returns CSS class name for each tab (lesson, examples, practice, test)

4. **Unit tests added**:
   - `test_tab_icon_returns_emoji_for_each_mode`
   - `test_tab_class_name_returns_css_class_for_each_mode`
   - `test_all_tab_modes_have_icons`
   - `test_all_tab_modes_have_class_names`

**Verification**:
- `cargo build --features cli` ✓
- `cargo test --lib module_tabs` ✓ (5 tests passed)

### [x] Task 1.3: Integrate Accordion into Learn Page
<!-- chat-id: d3d6617f-c8e7-4e29-9af3-c84dd788bb49 -->
**File**: `src/ui/pages/learn.rs`

**Completed**: Integrated mobile tab navigation into Learn page:

1. **Desktop tabs unchanged**: Existing `.content-tabs` horizontal tabs preserved
2. **CSS media query added**: Hide `.content-tabs` on mobile (`@media max-width: 768px`)
3. **New mobile tab styles**: Added `.mobile-content-tabs` with stacked vertical buttons
   - Full-width touch-friendly buttons (48px min-height)
   - Color-coded active states matching desktop (blue lesson, purple examples, green practice, yellow test)
   - Emoji icons for visual clarity (📖 📡 ✏️ 📝)
   - Touch-optimized with `-webkit-tap-highlight-color: transparent`
4. **Same content rendering**: Content panels remain unchanged, displayed below both tab types
5. **State wiring**: Mobile tabs wire to same `content_view` signal as desktop tabs

**Verification**:
- `cargo build --features cli` ✓
- `cargo test -- --skip e2e` ✓ (all tests pass)

### [ ] Task 1.4: Test Learn Page Mobile Tabs
<!-- chat-id: 171322b3-ab5d-4a8c-bc2a-52fdb78e509b -->
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
