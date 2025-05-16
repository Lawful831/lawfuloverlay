# Lawful Overlay

A Python-based overlay application designed to enhance your screen with real-time information and notifications.

## Features

- Real-time information tracking
- Customizable overlay interface
- Hotkey support for quick actions
- Cross-application compatibility

## Prerequisites

- Python 3.8 or higher
- Windows operating system
- Required Python packages (will be installed automatically)

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
   python main.py
   ```

2. The overlay will appear on your screen. You can:
   - Move it to any position by dragging the window
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