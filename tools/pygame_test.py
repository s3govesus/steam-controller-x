#!/usr/bin/env python3
"""Quick sanity check for the sc-adapter virtual gamepad, using pygame's
SDL2 joystick backend -- the same backend most native Linux games use.

Run `sc-adapter` first (e.g. `cargo run -p sc-adapter -- --pid 0x1302`) so
the virtual Xbox-360-shaped uinput device exists, then run this script and
press buttons / move sticks and triggers to confirm they show up correctly.

Note: this monitors *every* joystick SDL sees, not just one. Two things
can make more than one show up:
  - SDL's built-in HIDAPI Steam Controller driver may independently grab
    the real physical controller's raw HID interface and mis-parse it as
    its own ~22-button "Steam Controller" device (it doesn't know this new
    hardware's real layout) -- separate from sc-adapter's own virtual pad.
    Set `SDL_JOYSTICK_HIDAPI_STEAM=0` in the environment to suppress this.
  - Any other real Xbox-360-compatible controller connected at the same
    time will report the identical name/VID/PID we spoof, so name
    matching can't reliably tell them apart.
Since every index is monitored and tagged in the output, you can just
watch which index reacts to confirm which one is sc-adapter's.

Ctrl+C or close the tiny window to quit.
"""

import sys

import pygame

# Xbox 360 pad button index -> friendly name (SDL's standard mapping,
# which is what sc-adapter's uinput backend advertises).
BUTTON_NAMES = {
    0: "A",
    1: "B",
    2: "X",
    3: "Y",
    4: "LB",
    5: "RB",
    6: "Back",
    7: "Start",
    8: "Guide",
    9: "LThumb",
    10: "RThumb",
}

AXIS_NAMES = {
    0: "LeftX",
    1: "LeftY",
    2: "LeftTrigger",
    3: "RightX",
    4: "RightY",
    5: "RightTrigger",
}

AXIS_DEADZONE = 0.05


def wait_for_joysticks(timeout_polls=50):
    for _ in range(timeout_polls):
        pygame.event.pump()
        if pygame.joystick.get_count() > 0:
            return
        pygame.time.wait(100)


def main():
    pygame.init()
    pygame.joystick.init()
    # A tiny window keeps SDL's event pump reliable across platforms; it
    # doesn't need to be interacted with.
    pygame.display.set_mode((320, 120))
    pygame.display.set_caption("sc-adapter pygame test")

    print("Looking for joysticks/gamepads...")
    wait_for_joysticks()

    count = pygame.joystick.get_count()
    if count == 0:
        print("No joystick found after 5s. Is sc-adapter running?")
        sys.exit(1)

    joysticks = []
    print(f"\nFound {count} device(s):")
    for i in range(count):
        j = pygame.joystick.Joystick(i)
        j.init()
        joysticks.append(j)
        shape = f"buttons={j.get_numbuttons()} axes={j.get_numaxes()} hats={j.get_numhats()}"
        looks_right = j.get_numbuttons() == 11 and j.get_numaxes() == 6 and j.get_numhats() == 1
        tag = "  <- matches sc-adapter's shape" if looks_right else ""
        print(f"  [{i}] {j.get_name()!r} {shape}{tag}")

    if count > 1:
        print(
            "\nMore than one device found. Press buttons on the physical "
            "controller and watch which index [N] reacts below to confirm "
            "which one is sc-adapter's virtual pad (see this script's "
            "module docstring for why duplicates can show up)."
        )
    print("\nPress buttons / move sticks & triggers. Ctrl+C to quit.\n")

    last_buttons = [[False] * j.get_numbuttons() for j in joysticks]
    last_axes = [[0.0] * j.get_numaxes() for j in joysticks]
    last_hats = [(0, 0)] * len(joysticks)

    clock = pygame.time.Clock()
    try:
        while True:
            for event in pygame.event.get():
                if event.type == pygame.QUIT:
                    return

            for idx, joy in enumerate(joysticks):
                for i in range(joy.get_numbuttons()):
                    pressed = joy.get_button(i)
                    if pressed != last_buttons[idx][i]:
                        name = BUTTON_NAMES.get(i, f"button{i}")
                        print(f"[{idx}] {name:10s} {'DOWN' if pressed else 'up'}")
                        last_buttons[idx][i] = pressed

                for i in range(joy.get_numaxes()):
                    value = joy.get_axis(i)
                    if abs(value - last_axes[idx][i]) > AXIS_DEADZONE:
                        name = AXIS_NAMES.get(i, f"axis{i}")
                        print(f"[{idx}] {name:12s} {value:+.2f}")
                        last_axes[idx][i] = value

                if joy.get_numhats() > 0:
                    hat = joy.get_hat(0)
                    if hat != last_hats[idx]:
                        print(f"[{idx}] DPad         {hat}")
                        last_hats[idx] = hat

            clock.tick(60)
    except KeyboardInterrupt:
        pass
    finally:
        pygame.quit()
        print("\nbye")


if __name__ == "__main__":
    main()
