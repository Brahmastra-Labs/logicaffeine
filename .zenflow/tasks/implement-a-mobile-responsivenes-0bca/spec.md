# Technical Specification: Mobile Responsiveness

## Overview

This specification defines the implementation approach for making the LOGICAFFEINE website fully mobile-responsive, with a focus on the Learn page tab buttons that currently run off the screen on mobile devices.

## Technical Context

### Framework & Stack
- **UI Framework**: Dioxus 0.6 (Rust-based WASM framework)
- **Styling**: Inline CSS-in-Rust strings (no external CSS framework)
- **Existing Infrastructure**: `src/ui/responsive.rs` provides centralized mobile utilities

### Current Architecture
The codebase already has a well-designed responsive module (`responsive.rs`) with:
- Standard breakpoints (XS: 480px, SM: 640px, MD: 768px, LG: 1024px, XL: 1280px)
- WCAG-compliant touch targets (44px minimum)
- Mobile tab bar styles
- Panel switching patterns
- Safe area support for notched devices

### Problem Statement
1. **Learn Page Tabs**: The content tabs (Lesson, Examples, Practice, Test) use horizontal `display: flex` without overflow handling, causing buttons to run off-screen on mobile
2. **Inconsistent Breakpoints**: Different pages use different breakpoints (700px, 980px, 900px) instead of the standard ones in `responsive.rs`
3. **Missing Mobile Optimizations**: Several pages lack explicit mobile breakpoints for small devices (<480px)

## Implementation Approach

### Design Decision: Collapsible Accordion Pattern for Learn Page

Rather than horizontal tabs that overflow, the Learn page will use a **stacked accordion pattern** on mobile:

```
┌─────────────────────────────┐
│ 📖 LESSON                 ▼ │  <- Tappable header
├─────────────────────────────┤
│ [Lesson content here...]    │  <- Expanded section
│                             │
└─────────────────────────────┘
┌─────────────────────────────┐
│ 📝 EXAMPLES               ▶ │  <- Collapsed
└─────────────────────────────┘
┌─────────────────────────────┐
│ ∞ PRACTICE                ▶ │  <- Collapsed
└─────────────────────────────┘
┌─────────────────────────────┐
│ 📋 TEST                   ▶ │  <- Collapsed
└─────────────────────────────┘
```

**Rationale**:
- Full-width touch targets meet 44px minimum
- Content visible without horizontal scrolling
- Familiar mobile pattern users understand
- Progressive disclosure reduces cognitive load
- Works in both portrait and landscape orientations

### Alternative Considered: Scrollable Tab Bar
A horizontally scrolling tab bar was considered but rejected because:
- Users may not discover hidden tabs
- Requires precise horizontal swipe gestures
- Less intuitive for the educational content structure

## Source Code Structure Changes

### Files to Modify

| File | Change Type | Description |
|------|-------------|-------------|
| `src/ui/pages/learn.rs` | Major | Add accordion styles, refactor tab UI for mobile |
| `src/ui/components/module_tabs.rs` | Major | Create mobile accordion variant |
| `src/ui/responsive.rs` | Minor | Add accordion component styles |
| `src/ui/pages/landing.rs` | Minor | Standardize breakpoints |
| `src/ui/pages/pricing.rs` | Minor | Standardize breakpoints, add XS support |
| `src/ui/pages/workspace.rs` | Minor | Add tablet/phone breakpoints |
| `src/ui/pages/lesson.rs` | Minor | Add explicit mobile breakpoints |
| `src/ui/pages/profile.rs` | Minor | Add explicit mobile breakpoints |
| `src/ui/components/main_nav.rs` | Minor | Add hamburger menu for <640px |

### New Components

#### MobileAccordionTabs Component
Location: `src/ui/components/module_tabs.rs` (extend existing)

```rust
/// Mobile accordion variant of tab navigation
/// Shows on screens <768px, hidden on desktop
#[component]
pub fn MobileAccordionTabs(props: ModuleTabsProps) -> Element {
    // Stacked, full-width accordion buttons
    // Only one section expanded at a time
    // Smooth expand/collapse animations
}
```

### CSS Architecture

#### New Accordion Styles (in responsive.rs)
```css
/* Mobile Accordion Tabs - shown only on mobile */
.accordion-tabs {
    display: none;  /* Hidden on desktop */
}

@media (max-width: 768px) {
    .accordion-tabs {
        display: flex;
        flex-direction: column;
        gap: 8px;
        width: 100%;
    }

    .accordion-tab-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        min-height: 48px;  /* Touch-friendly */
        padding: 12px 16px;
        background: rgba(255, 255, 255, 0.05);
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 10px;
        cursor: pointer;
        -webkit-tap-highlight-color: transparent;
    }

    .accordion-tab-content {
        max-height: 0;
        overflow: hidden;
        transition: max-height 0.3s ease;
    }

    .accordion-tab-content.expanded {
        max-height: 2000px;  /* Large enough for content */
    }

    /* Hide horizontal tabs on mobile */
    .content-tabs {
        display: none !important;
    }
}
```

## Data Model / API / Interface Changes

No backend changes required. This is purely a frontend styling and component update.

### Component Interface Updates

#### ModuleTabsProps (extended)
```rust
#[derive(Props, Clone, PartialEq)]
pub struct ModuleTabsProps {
    current: TabMode,
    on_change: EventHandler<TabMode>,
    #[props(default)]
    locked_tabs: Vec<TabMode>,
    #[props(default = false)]
    compact: bool,
    // NEW: Force accordion layout regardless of screen size (for testing)
    #[props(default = false)]
    force_accordion: bool,
}
```

## Delivery Phases

### Phase 1: Learn Page Mobile Fix (Priority)
**Goal**: Fix the immediate UX problem with tabs running off-screen

1. Add accordion tab styles to `responsive.rs`
2. Create `MobileAccordionTabs` component variant
3. Update Learn page to render accordion on mobile, horizontal tabs on desktop
4. Test on iOS Safari, Android Chrome, and desktop browsers

**Verification**:
- Tabs visible and tappable on 320px width viewport
- Touch targets meet 44px minimum
- Content expands/collapses smoothly
- Desktop layout unchanged

### Phase 2: Standardize Breakpoints Site-Wide
**Goal**: Consistent responsive behavior across all pages

1. Update `landing.rs` to use standard breakpoints (768px, 480px)
2. Update `pricing.rs` to use standard breakpoints
3. Update `workspace.rs` with tablet breakpoints
4. Add mobile breakpoints to `lesson.rs` and `profile.rs`

**Verification**:
- All pages use breakpoints from `responsive.rs`
- No horizontal scrolling on any page at 320px width
- Grids collapse appropriately on small screens

### Phase 3: Navigation Enhancement
**Goal**: Professional mobile navigation

1. Add hamburger menu component
2. Implement slide-out navigation drawer for <640px
3. Update main_nav.rs to use hamburger menu pattern

**Verification**:
- All navigation accessible on mobile
- Menu opens/closes with animation
- Links work correctly from drawer

### Phase 4: Polish & Optimization
**Goal**: Production-ready mobile experience

1. Audit all touch targets site-wide
2. Add landscape orientation optimizations
3. Implement safe-area insets for notched devices
4. Performance testing on low-end mobile devices

**Verification**:
- Lighthouse mobile score > 90
- All interactive elements meet WCAG 2.5.5 (44px targets)
- No layout shifts on orientation change

## Verification Approach

### Testing Strategy

#### Unit Tests (Rust)
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_accordion_renders_all_tabs() {
        // Verify all TabMode variants rendered
    }

    #[test]
    fn test_locked_tabs_not_expandable() {
        // Verify locked tabs show lock icon and don't respond to clicks
    }
}
```

#### Manual Testing Checklist
- [ ] iPhone SE (320px) - smallest common viewport
- [ ] iPhone 14 Pro (393px) - notched device with safe areas
- [ ] iPad Mini (768px) - tablet breakpoint boundary
- [ ] Samsung Galaxy S21 (360px) - common Android size
- [ ] Desktop Firefox, Chrome, Safari (unchanged behavior)

#### CSS Verification Commands
```bash
# Build and serve locally
cd logos && cargo build --features cli
# Then open browser dev tools and test responsive modes
```

### Acceptance Criteria

1. **Learn Page Tabs**: All 4 tabs (Lesson, Examples, Practice, Test) visible and tappable on 320px viewport
2. **Touch Targets**: All buttons/links minimum 44x44px on mobile
3. **No Horizontal Scroll**: No page requires horizontal scrolling at 320px width
4. **Performance**: No janky animations, smooth 60fps transitions
5. **Accessibility**: Screen reader announces accordion state (expanded/collapsed)
6. **Backward Compatibility**: Desktop experience unchanged

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| CSS conflicts with existing styles | Medium | Low | Use specific `.mobile-` prefixed classes |
| Animation performance on old devices | Low | Medium | Use CSS transitions, not JS animations |
| Safari-specific bugs | Medium | Medium | Test early, use webkit prefixes |
| Content too long for accordion | Low | Low | Set reasonable max-height, add scroll within |

## Dependencies

- No external package dependencies
- Relies on existing `responsive.rs` infrastructure
- Dioxus 0.6 signal/hook system for accordion state

## Open Questions

1. Should the hamburger menu be implemented as a separate component or integrated into main_nav.rs? **Recommendation**: Separate component for reusability
2. Should accordion state persist when navigating between modules? **Recommendation**: No, default to Lesson tab expanded
3. Should we add swipe gestures for tab switching? **Recommendation**: Not for Phase 1, consider for future enhancement
