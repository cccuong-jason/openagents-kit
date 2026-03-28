# OpenAgents First-Run UX Refresh Design

## Summary

Refresh the first-run terminal experience so OpenAgents Kit feels like it actively sets up the workspace for the user instead of exposing a raw control surface. The new experience should feel distinct from Claude Code while preserving the same sense of polish: a cool-toned pixel-console identity, a more intentional mascot, a short boot scan, and a guided onboarding wizard with one clear action per screen.

## Product Goals

- Make the first-run promise feel like "setup for me".
- Guide both technical and non-technical users toward the next action without guessing controls.
- Distinguish OpenAgents visually from Claude Code.
- Keep the existing detection-first onboarding architecture and generated manifest flow.

## Experience Direction

### Core flow

The first-run flow becomes:

`Boot -> Detect -> Recommend -> Guided Adjustments (optional) -> Generate -> Next Steps`

- `Boot` shows a short scanning/loading state with no required input.
- `Detect` is narrated in plain language so the user understands what OpenAgents is checking.
- `Recommend` presents the detected tools and a recommended setup with one primary action.
- `Guided Adjustments` becomes a step-based wizard instead of a dense inline editor.
- `Generate` confirms what is being written.
- `Next Steps` clearly tells the user what to do after generation.

### Audience balance

The default path should be fast for technical users and reassuring for non-technical users:

- Technical users can accept the recommendation immediately.
- Non-technical users can move through guided steps one decision at a time.

## Visual Direction

### Mascot

Replace the current generic ASCII face with a defined 8-bit operator bot:

- square body
- two top antenna pixels
- visor/eye band
- short side arms / grounded stance

The mascot should support three states with the same silhouette:

- `idle`
- `scanning`
- `ready`

### Palette

Replace the warm orange palette with a cool retro console palette:

- primary accent: neon teal
- progress/success accent: electric lime
- secondary text and chrome: slate blue-gray
- body text: soft ivory on near-black

This should feel playful and pixel-native without becoming noisy or toy-like.

## Interaction Model

### Recommendation screen

The recommendation screen should answer:

- what OpenAgents found
- what OpenAgents recommends
- what happens if the user accepts
- how to refine if they want changes

Primary action:

- `Enter` accepts the recommended setup

Secondary action:

- `Tab` enters guided adjustments

Exit:

- `Esc` or `q`

### Guided wizard

Guided adjustments become a short wizard with explicit steps and progress markers:

1. profile preset
2. memory backend
3. enabled tools
4. review

Wizard controls should be screen-specific and stated in plain language in the footer.

### Next-step guidance

Every setup state must include a clear action statement, especially after generation. The user should always be told what OpenAgents is doing or what they should do next.

## Technical Design

### TUI state model

Extend the setup TUI state to support:

- a boot/loading screen
- a recommendation screen
- a guided wizard with step tracking
- a completion screen

The TUI should keep the existing detection and manifest-generation plumbing and only change how the setup state is presented and navigated.

### Rendering strategy

Keep the current single-file TUI implementation if practical, but extract small helper functions for:

- screen titles and prompts
- control text
- mascot/state rendering
- wizard step rendering

This keeps the screen logic testable without forcing a wider refactor.

### Boot animation

The boot screen should be time-based and lightweight:

- pulse loading dots
- simple scanning copy
- auto-advance after a short interval

No spinner library or external dependency is needed.

## Testing

Add unit coverage for the new setup flow helpers:

- screen transition defaults
- guided wizard step labels and control text
- recommendation CTA copy
- mascot/state or loading copy helpers where practical

Keep existing setup/application tests passing so the UX refresh does not regress manifest generation or adapter outputs.
