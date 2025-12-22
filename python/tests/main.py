import random

A_BUTTON = 0x8000
B_BUTTON = 0x4000
L_TRIG = 0x0020
R_TRIG = 0x0010
Z_TRIG = 0x2000
START_BUTTON = 0x1000
U_JPAD = 0x0800
L_JPAD = 0x0200
R_JPAD = 0x0100
D_JPAD = 0x0400
U_CBUTTONS = 0x0008
L_CBUTTONS = 0x0002
R_CBUTTONS = 0x0001
D_CBUTTONS = 0x0004


import struct

import numpy as np
import matplotlib.pyplot as plt
from mpl_toolkits.mplot3d import Axes3D
import imageio
import os


from gameplay import GAMEPLAY
from sm64_binds import SM64Game, SM64GameGenerator, GamePad

def create_3d_rotation_gif(positions, points_per_frame=1, filename='3d_rotation.gif'):
    frames = []
    
    for i in range(0, len(positions), points_per_frame):
        fig = plt.figure()
        ax = fig.add_subplot(111, projection='3d')

        # Plot the points for the current frame
        ax.scatter(positions[:i + points_per_frame, 0], positions[:i + points_per_frame, 2], positions[:i + points_per_frame, 1], c='b', marker='o', s=4)

        # Set limits
        ax.set_xlim([-8000, 8000])
        ax.set_ylim([-8000, 8000])
        ax.set_zlim([-8000, 8000])

        # Rotate the view
        ax.view_init(30, (i // points_per_frame) * 5 % 360)  # Adjust rotation speed and angle

        # Save the current frame to a list
        plt.savefig('temp.png')
        plt.close(fig)
        frames.append(imageio.imread('temp.png'))

    # Clean up temporary images
    os.remove('temp.png')

    # Save all frames as a GIF
    imageio.mimsave(filename, frames, fps=30)

def run():
    # ----------------------------------
    # Load ROM and initialize game
    # ----------------------------------
    rom_bytes = open("./baserom.us.z64", "rb").read()

    generator = SM64GameGenerator.new(rom_bytes)
    game = generator.create_game()

    # ----------------------------------
    # gameplay loop
    # ----------------------------------
    positions_list = []

    # for i in range(len(GAMEPLAY.keys()) // 4):
    for i in range(5000):

        # Extract controller bytes from GAMEPLAY dict
        # b = bytes([GAMEPLAY[str(n)] for n in range(4*i, 4*i+4)])

        # # Parse inputs
        # button = (b[0] << 8) | b[1]
        # stickx = int(b[2])
        # sticky = int(b[3])

        # ----------------------------------
        # Override logic (same as your script)
        # ----------------------------------
        pad = GamePad(0, 0, 0)

        if 150 < i < 160 or 200 < i < 300:
            pad.button = START_BUTTON

        if i > 300:
            pad.stick_y = 80

        if i % 2 == 0:
            pad.button = A_BUTTON


        # Step
        game.step_game(pad)

        # Read state
        state = game.get_game_state()
        positions_list.append(state.pos)
        print(state.pos)

    # ----------------------------------
    # Convert to array and animate
    # ----------------------------------
    points = np.array(positions_list)
    create_3d_rotation_gif(points, points_per_frame=100)

# ---------------------------------------------------------

if __name__ == '__main__':
    run()
