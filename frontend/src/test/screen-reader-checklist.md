# Screen Reader Checklist

Run the same smoke pass against a production build on each supported platform.

## NVDA

- Start at the workbench and verify the title bar, activity rails, panels, editor, and status bar have useful landmarks and names.
- Open and close a modal with the keyboard; verify focus enters the modal, remains inside it, and returns to the trigger.
- Change a setting and verify the result is announced without moving focus unexpectedly.
- Trigger an error, warning, and success notification; verify each is announced once with its severity.
- Navigate the command palette and menus with Tab, arrows, Enter, Space, and Escape.

## VoiceOver

- Repeat the workbench landmark and modal checks in Safari with Quick Nav both enabled and disabled.
- Verify rotor headings, landmarks, buttons, form controls, and dialog entries have meaningful labels.
- Verify live regions announce stream updates and notifications without interrupting text entry.
- Verify Escape closes transient surfaces and restores focus to the invoking control.

## Orca

- Repeat the workbench landmark, modal, notification, and command-palette checks in the Linux desktop session.
- Verify focus order follows the visual order and arrow-key navigation works in composite controls.
- Verify disabled controls are skipped and selected, expanded, and busy states are announced.
- Verify the application remains usable at 200% text scaling and with a high-contrast theme.