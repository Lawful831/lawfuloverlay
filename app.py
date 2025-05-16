import tkinter as tk
from tkinter import simpledialog, messagebox, font as tkFont # Added tkFont
import discord
import threading
import asyncio
import queue # For thread-safe communication

# --- Configuration ---
BOT_TOKEN = None
TARGET_USER_ID = None
TARGET_SERVER_ID = None

# --- Style Configuration for Undertale Textbox ---
OVERLAY_WIDTH = 450       # Width of the entire overlay box
OVERLAY_HEIGHT = 120      # Height of the entire overlay box
BORDER_THICKNESS = 4      # Thickness of the black border
TEXT_PADDING = 10         # Padding around the text inside the white box
# --- FONT ---
# Friend should try to install "Determination Mono" (search online for the TTF)
# If not available, change to "Fixedsys", "Courier New", or other installed pixel/mono font
FONT_FAMILY = "Determination Mono" 
FONT_SIZE = 18            # Adjust size as needed for the chosen font
FALLBACK_FONT_FAMILY = "Courier New" # If primary font fails

# --- Global Variables ---
message_queue = queue.Queue()
discord_bot_thread = None
bot_instance = None
overlay_root = None

# --- Discord Bot Class (Unchanged from previous version) ---
class DiscordClient(discord.Client):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.target_user_id = TARGET_USER_ID
        self.target_server_id = TARGET_SERVER_ID
        self.message_queue = kwargs.get('message_queue')

    async def on_ready(self):
        print(f'Logged in as {self.user.name} (ID: {self.user.id})')
        print('------')
        server = self.get_guild(self.target_server_id)
        if server:
            print(f"Monitoring server: {server.name}")
        else:
            print(f"Error: Could not find server with ID {self.target_server_id}. Check the ID and bot's presence.")
            message_queue.put("Error: Server not found.")
        
        try:
            user = await self.fetch_user(self.target_user_id)
            if user:
                print(f"Monitoring user: {user.name}#{user.discriminator}")
            else:
                print(f"Warning: Could not initially fetch user with ID {self.target_user_id}. Will rely on message author ID.")
                message_queue.put("Warning: User ID might be incorrect.")
        except discord.NotFound:
            print(f"Warning: User with ID {self.target_user_id} not found. Will rely on message author ID.")
            message_queue.put("Warning: User ID might be incorrect or user not found.")
        except Exception as e:
            print(f"Error fetching user {self.target_user_id}: {e}")


    async def on_message(self, message):
        if message.author == self.user:
            return
        if message.guild and message.guild.id == self.target_server_id and \
           message.author.id == self.target_user_id:
            
            print(f"Message from target: {message.author.name}: {message.content}")
            if self.message_queue:
                # We only need the content for the overlay, name is implicit (it's you)
                # Or, if you want to show your name: f"{message.author.name}: {message.content}"
                self.message_queue.put(message.content)


# --- Overlay GUI Class ---
class OverlayWindow:
    def __init__(self, root, msg_queue):
        self.root = root
        self.message_queue = msg_queue
        self.current_message_text = "* Waiting for messages..." # Initial text

        # Window Setup
        self.root.title("Chat Overlay")
        
        # --- Transparency & Always on Top ---
        transparent_color = 'magenta' # This color will be made transparent
        self.root.attributes("-topmost", True)
        self.root.attributes("-transparentcolor", transparent_color)
        self.root.config(bg=transparent_color) # Set root background to the transparent color
        
        self.root.overrideredirect(True) # Remove window decorations
        
        # Set initial size and position
        screen_width = self.root.winfo_screenwidth()
        screen_height = self.root.winfo_screenheight()
        self.width = OVERLAY_WIDTH
        self.height = OVERLAY_HEIGHT
        # Default to top-left, user can drag it
        self.x_pos = 20 
        self.y_pos = 20
        self.root.geometry(f"{self.width}x{self.height}+{self.x_pos}+{self.y_pos}")

        # --- Font Selection ---
        try:
            self.pixel_font = tkFont.Font(family=FONT_FAMILY, size=FONT_SIZE)
            # Small check to see if Tkinter found the font or a close substitute
            if FONT_FAMILY.lower() not in self.pixel_font.actual("family").lower():
                print(f"Warning: Font '{FONT_FAMILY}' not found or matched. Trying fallback '{FALLBACK_FONT_FAMILY}'.")
                self.pixel_font = tkFont.Font(family=FALLBACK_FONT_FAMILY, size=FONT_SIZE)
        except tk.TclError:
            print(f"Error loading font '{FONT_FAMILY}'. Using system default.")
            self.pixel_font = tkFont.Font(family=FALLBACK_FONT_FAMILY, size=FONT_SIZE) # Fallback


        # --- Undertale Textbox Style ---
        # 1. Outer container (acts as the black border)
        self.border_frame = tk.Frame(self.root, bg="black")
        self.border_frame.pack(fill="both", expand=True)

        # 2. Inner label (the white text area)
        # Calculate wraplength for the text within the white box
        # Width of white area = OVERLAY_WIDTH - 2 * BORDER_THICKNESS
        # Wraplength = Width of white area - 2 * TEXT_PADDING
        self.text_wraplength = self.width - (2 * BORDER_THICKNESS) - (2 * TEXT_PADDING)

        self.message_label = tk.Label(
            self.border_frame, # Parent is the border_frame
            text=self.current_message_text,
            font=self.pixel_font,
            fg="black",  # Text color
            bg="white",  # Background of the text area
            wraplength=self.text_wraplength,
            justify="left",
            anchor="nw", # Anchor text to top-left
            padx=TEXT_PADDING, # Internal padding for text from white box edges
            pady=TEXT_PADDING
        )
        # Pack the white text area inside the black border_frame,
        # leaving BORDER_THICKNESS of black visible around it.
        self.message_label.pack(
            fill="both", 
            expand=True, 
            padx=BORDER_THICKNESS, 
            pady=BORDER_THICKNESS
        )

        # --- Close Button (Styled and placed on the border) ---
        self.close_button = tk.Button(
            self.border_frame, # Parent is the border_frame so it sits on black
            text="X",
            command=self.close_app,
            bg="black", # Button background same as border
            fg="white", # Text color for X
            font=("Arial", 8, "bold"), # Small, simple font for X
            relief="flat",
            borderwidth=0,
            highlightthickness=0
        )
        # Place it in the top-right corner of the border
        close_button_size = 18
        self.close_button.place(
            x=self.width - close_button_size - (BORDER_THICKNESS -1), 
            y=(BORDER_THICKNESS -1), 
            width=close_button_size, 
            height=close_button_size,
            anchor="ne" # Anchor to North-East of its x,y position
        )
        # Adjusting placement slightly for aesthetics.
        # The x,y for place is relative to the parent (border_frame).
        # We want it in the top-right black border area.
        self.close_button.place(
            relx=1.0, rely=0.0, # Top-right of parent
            x=-BORDER_THICKNESS/2, y=BORDER_THICKNESS/2, # Offset slightly into the border
            width=close_button_size, height=close_button_size,
            anchor="center" # Anchor by its center for precise cornering
        )


        # --- Make window draggable ---
        self._offset_x = 0
        self._offset_y = 0
        # Bind dragging to the border and the text area
        self.border_frame.bind("<ButtonPress-1>", self.on_press)
        self.border_frame.bind("<B1-Motion>", self.on_drag)
        self.message_label.bind("<ButtonPress-1>", self.on_press)
        self.message_label.bind("<B1-Motion>", self.on_drag)
        # Also allow dragging by the root window background IF it wasn't fully transparent
        # (but it is, so this binding is less critical here but good for other contexts)
        self.root.bind("<ButtonPress-1>", self.on_press)
        self.root.bind("<B1-Motion>", self.on_drag)


        # Start checking for messages
        self.update_message_display()

    def on_press(self, event):
        # Check if the event widget is the close button; if so, don't start drag
        if event.widget == self.close_button:
            return
        self._offset_x = event.x_root - self.root.winfo_x()
        self._offset_y = event.y_root - self.root.winfo_y()


    def on_drag(self, event):
        # Check if the event widget is the close button; if so, don't drag
        # This check might not be strictly necessary with how on_press is set up, but safe
        if event.widget == self.close_button:
            return
        x = event.x_root - self._offset_x
        y = event.y_root - self._offset_y
        self.root.geometry(f"+{x}+{y}")


    def update_message_display(self):
        try:
            while not self.message_queue.empty():
                new_msg_content = self.message_queue.get_nowait()
                if new_msg_content.startswith("Error:") or new_msg_content.startswith("Warning:"):
                    # Display errors/warnings directly without the '*'
                    self.current_message_text = new_msg_content
                else:
                    self.current_message_text = f"* {new_msg_content}"
                self.message_label.config(text=self.current_message_text)
                # Removed the auto-fade out logic to keep message visible
        except queue.Empty:
            pass
        
        self.root.after(100, self.update_message_display)

    def close_app(self):
        print("Close button clicked. Shutting down...")
        global bot_instance, discord_bot_thread, overlay_root
        
        if bot_instance and bot_instance.is_ready():
            print("Attempting to close Discord bot...")
            asyncio.run_coroutine_threadsafe(bot_instance.close(), bot_instance.loop)
        
        if discord_bot_thread and discord_bot_thread.is_alive():
            print("Waiting for Discord bot thread to join...")
            discord_bot_thread.join(timeout=5.0)
            if discord_bot_thread.is_alive():
                print("Bot thread did not join, continuing shutdown.")

        if overlay_root:
            print("Destroying Tkinter window...")
            overlay_root.destroy()
        
        print("Application shutdown complete.")

# --- Bot Thread Function (Unchanged) ---
def run_discord_bot(token, intents, msg_queue):
    global bot_instance
    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    
    bot_instance = DiscordClient(
        intents=intents, 
        message_queue=msg_queue,
        loop=loop # Pass the loop explicitly
    )
    try:
        bot_instance.run(token)
    except discord.LoginFailure:
        print("Login failed: Incorrect token or bot permissions.")
        msg_queue.put("Error: Discord Login Failed.")
    except Exception as e:
        print(f"An error occurred in the Discord bot thread: {e}")
        msg_queue.put(f"Bot Error: {e}")
    finally:
        print("Discord bot thread finished.")

# --- Main Application Logic (Unchanged except for IDs type) ---
def main():
    global BOT_TOKEN, TARGET_USER_ID, TARGET_SERVER_ID, discord_bot_thread, overlay_root

    root_config = tk.Tk()
    root_config.withdraw()
    
    if not BOT_TOKEN:
        BOT_TOKEN = simpledialog.askstring("Bot Token", "Enter your Discord Bot Token:", parent=root_config)
        if not BOT_TOKEN:
            messagebox.showerror("Error", "Bot Token is required.")
            root_config.destroy()
            return

    if not TARGET_USER_ID:
        user_id_str = simpledialog.askstring("Target User ID", "Enter YOUR Discord User ID (the message sender's ID):", parent=root_config)
        if not user_id_str or not user_id_str.isdigit():
            messagebox.showerror("Error", "Valid Target User ID is required.")
            root_config.destroy()
            return
        TARGET_USER_ID = int(user_id_str)

    if not TARGET_SERVER_ID:
        server_id_str = simpledialog.askstring("Target Server ID", "Enter the Target Server ID:", parent=root_config)
        if not server_id_str or not server_id_str.isdigit():
            messagebox.showerror("Error", "Valid Target Server ID is required.")
            root_config.destroy()
            return
        TARGET_SERVER_ID = int(server_id_str)
    
    root_config.destroy()

    intents = discord.Intents.default()
    intents.guilds = True
    intents.messages = True
    intents.message_content = True

    print("Starting Discord bot thread...")
    discord_bot_thread = threading.Thread(
        target=run_discord_bot, 
        args=(BOT_TOKEN, intents, message_queue),
        daemon=True
    )
    discord_bot_thread.start()

    print("Starting Tkinter overlay GUI...")
    overlay_root = tk.Tk()
    app = OverlayWindow(overlay_root, message_queue)
    overlay_root.protocol("WM_DELETE_WINDOW", app.close_app)
    overlay_root.mainloop()

    if discord_bot_thread and discord_bot_thread.is_alive():
        print("Mainloop finished, ensuring bot thread cleanup...")
        if bot_instance and bot_instance.loop and not bot_instance.loop.is_closed():
             asyncio.run_coroutine_threadsafe(bot_instance.close(), bot_instance.loop)
        discord_bot_thread.join(timeout=5)

if __name__ == "__main__":
    print("Discord Chat Overlay Application - Undertale Style")
    print("-------------------------------------------------")
    print("INFO: For the best Undertale look, try to install the 'Determination Mono' font.")
    print("      You can find it by searching online (e.g., on DaFont).")
    print("      If not found, a fallback font will be used.")
    print("      You can also edit FONT_FAMILY in the script.")
    print("-------------------------------------------------\n")
    # (Instructions for Bot Token, User ID, Server ID remain the same)
    print("You will be asked for:")
    print("1. A Discord Bot Token (your friend who is setting this up needs to provide this).")
    print("2. The User ID of the person whose messages you want to see (your friend's Discord ID).")
    print("3. The Server ID where they will be chatting.")
    print("\nHow to get IDs:")
    print("  - Enable Developer Mode in Discord: User Settings > App Settings > Advanced > Developer Mode (toggle on).")
    print("  - To get User ID: Right-click a user's name > Copy ID.")
    print("  - To get Server ID: Right-click a server icon > Copy ID.")
    print("\nBot Setup (for the person whose messages are shown):")
    print("  - Create a Bot Application: https://discord.com/developers/applications")
    print("  - Go to 'Bot' tab, click 'Add Bot'.")
    print("  - Copy the TOKEN (keep it secret!).")
    print("  - Enable 'MESSAGE CONTENT INTENT' under 'Privileged Gateway Intents'.")
    print("  - Invite the bot to the server: Go to 'OAuth2' > 'URL Generator'. Select 'bot' scope. ")
    print("    Under 'Bot Permissions', select 'View Channels' and 'Read Message History/Read Messages'.")
    print("    Copy the generated URL and paste it into a browser to invite the bot.")
    print("--------------------------------\n")
    
    main()