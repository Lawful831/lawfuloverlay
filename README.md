# Lawful Overlay

A Python-based Discord chat overlay application designed to enhance your screen with real-time messages from Discord.

## Features

- Real-time Discord chat messages display
- Customizable Undertale-style interface
- Hotkey support for quick actions
- Cross-application compatibility
- Discord bot integration

## Prerequisites

- Python 3.8 or higher
- Windows operating system
- Discord bot token (see Discord Setup below)
- Required Python packages (will be installed automatically)

## Discord Setup

1. Create a Discord bot:
   - Go to https://discord.com/developers/applications
   - Click "New Application"
   - Give your application a name
   - Go to "Bot" tab and click "Add Bot"
   - Copy your bot token

2. Configure your bot:
   - Go to "OAuth2" → "URL Generator"
   - Select "bot" scope
   - Add these permissions: "Read Messages", "Read Message History"
   - Copy the generated URL and invite the bot to your server


## Installation

1. Clone this repository to your local machine
2. Open a terminal in the project directory
3. Install the required dependencies:
   ```bash
   pip install -r requirements.txt
   ```

## Usage

1. Launch the overlay application:
   ```bash
   python app.py
   ```

2. The overlay will appear on your screen. You can:
   - Move it to any position by dragging the window
   - Resize it using the corners
   - Configure settings through the settings menu

3. The overlay will display messages from your Discord server in an Undertale-style text box.
   - Resize it using the corners
   - Configure settings through the settings menu

3. Use the default hotkeys:
   - `Ctrl + Shift + O`: Toggle overlay visibility
   - `Ctrl + Shift + S`: Open settings menu
   - `Ctrl + Shift + Q`: Quit the application

## Configuration

The application can be configured through:
1. The settings menu
2. The `config.json` file in the application directory

## Troubleshooting

- If the overlay doesn't appear, ensure you have administrator privileges
- If hotkeys aren't working, check for conflicts with other applications
- If you encounter any errors, please check the logs in the `logs` directory

## Contributing

1. Fork the repository
2. Create your feature branch
3. Commit your changes
4. Push to the branch
5. Create a new Pull Request

## License

This project is licensed under the MIT License - see the LICENSE file for details.