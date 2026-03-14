"""
LawfulOverlay — Cliente de Overlay para Discord
Se conecta al servidor LawfulOverlay vía WebSocket y muestra los mensajes
como un overlay transparente siempre visible.

Nota de privacidad
──────────────────
El modo "Detección automática" lee un pequeño cuadrado de píxeles (por defecto
60×60 px) directamente bajo el overlay para elegir un color de texto legible.
Esos datos nunca se almacenan ni se envían a ningún sitio.
El token del bot NO se gestiona aquí; todas las credenciales viven en el servidor.
"""

import tkinter as tk
from tkinter import colorchooser, messagebox, ttk
import threading
import asyncio
import queue
import json
from pathlib import Path
from PIL import ImageGrab
import numpy as np
import websockets

# ── Constantes de la barra de título ──────────────────────────────────────
TRANSPARENT_KEY = "magenta"   # El gestor de ventanas hace este color transparente
FRAME_H         = 28          # Altura de la barra de título en px
CHROME_BG       = "#1E1F22"
CHROME_FG       = "#BABBBE"

# ── Configuración por defecto ──────────────────────────────────────────────
DEFAULTS: dict = {
    # Conexión
    "ws_url":           "ws://127.0.0.1:8765",

    # Pantalla
    "overlay_w":        440,
    "overlay_h":        110,
    "always_on_top":    True,

    # Fuente
    "font_family":      "Arial",
    "font_size":        14,
    "font_bold":        True,

    # Estilo
    "bg_style":         "badge",    # "none" | "badge" | "auto_badge"
    "badge_color":      "#111827",
    "badge_padding":    10,
    "badge_radius":     8,
    "text_color":       "#FFFFFF",
    "outline_color":    "#000000",
    "outline_width":    2,

    # Detección
    "auto_detect":      True,
    "detect_interval":  2500,       # ms entre muestras de píxeles
    "detect_sample_px": 60,         # lado del cuadrado de muestra en px
    "detect_threshold": 25,         # delta mínimo por canal para actualizar el color
}

cfg: dict = dict(DEFAULTS)
_msg_q: queue.Queue = queue.Queue()


# ── Persistencia de configuración ─────────────────────────────────────────
def _cfg_path() -> Path:
    d = Path.home() / ".discord_chat_overlay"
    d.mkdir(exist_ok=True)
    return d / "client_settings.json"


def load_cfg() -> None:
    p = _cfg_path()
    if not p.exists():
        return
    try:
        with open(p) as f:
            saved = json.load(f)
        cfg.update({k: saved[k] for k in DEFAULTS if k in saved})
        print(f"[cfg] Cargado desde {p}")
    except Exception as e:
        print(f"[cfg] Error al cargar: {e}")


def save_cfg() -> None:
    p = _cfg_path()
    try:
        with open(p, "w") as f:
            json.dump(cfg, f, indent=2)
        print(f"[cfg] Guardado en {p}")
    except Exception as e:
        print(f"[cfg] Error al guardar: {e}")


# ── Utilidades de color ───────────────────────────────────────────────────
def _srgb(c: float) -> float:
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def relative_luminance(r: int, g: int, b: int) -> float:
    """Luminancia relativa según WCAG 2.1 (0 = negro, 1 = blanco)."""
    return 0.2126 * _srgb(r / 255) + 0.7152 * _srgb(g / 255) + 0.0722 * _srgb(b / 255)


def contrast_ratio(l1: float, l2: float) -> float:
    lo, hi = min(l1, l2), max(l1, l2)
    return (hi + 0.05) / (lo + 0.05)


def wcag_best_pair(bg_lum: float) -> tuple[str, str]:
    """
    Devuelve (color_texto, color_contorno) con mayor contraste WCAG 2.1
    respecto a bg_lum.  Garantiza al menos 4.5:1 (nivel AA).
    """
    white_cr = contrast_ratio(bg_lum, 1.0)
    black_cr = contrast_ratio(bg_lum, 0.0)
    if white_cr >= black_cr:
        return "#FFFFFF", "#111111"
    return "#111111", "#FFFFFF"


def auto_badge_color(bg_lum: float) -> str:
    """Fondo oscuro → badge casi negro.  Fondo claro → badge casi blanco."""
    return "#0D1117" if bg_lum > 0.4 else "#F0F0F0"


def hex2rgb(h: str) -> tuple[int, int, int]:
    h = h.lstrip("#")
    return int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)


def lerp_hex(a: str, b: str, t: float) -> str:
    """Interpola linealmente entre dos colores hexadecimales."""
    r1, g1, b1 = hex2rgb(a)
    r2, g2, b2 = hex2rgb(b)
    return "#{:02x}{:02x}{:02x}".format(
        int(r1 + (r2 - r1) * t),
        int(g1 + (g2 - g1) * t),
        int(b1 + (b2 - b1) * t),
    )


def sample_bg(x: int, y: int, size: int) -> tuple[int, int, int] | None:
    """
    Captura un pequeño cuadrado de píxeles bajo el overlay y devuelve la
    mediana RGB.  Es la ÚNICA lectura de pantalla que realiza esta aplicación.
    Los datos se usan solo para calcular un color y se descartan inmediatamente.
    """
    try:
        img = ImageGrab.grab(bbox=(x, y, x + size, y + size))
        arr = np.asarray(img)[:, :, :3].reshape(-1, 3)
        return tuple(np.median(arr, axis=0).astype(int))
    except Exception as e:
        print(f"[detección] error de muestra: {e}")
        return None


# ── Renderizador Canvas ────────────────────────────────────────────────────
class OverlayCanvas(tk.Canvas):
    """
    Canvas transparente que dibuja un badge opcional + texto con contorno.

    Diseño:
      • fondo = TRANSPARENT_KEY → invisible a través del gestor de ventanas
      • badge: rectángulo redondeado dibujado en el canvas (sólido o tramado)
      • texto: 8 copias desplazadas en outline_color + 1 principal en text_color

    El rectángulo redondeado se construye con rectángulos + arcos (sin
    smooth=True), evitando así que los píxeles interpolados de la B-spline
    aparezcan como color magenta/morado visible.
    """
    _OFFSETS = [(-1, -1), (0, -1), (1, -1), (-1, 0),
                (1, 0), (-1, 1), (0, 1), (1, 1)]

    def __init__(self, parent: tk.Widget) -> None:
        super().__init__(parent, bg=TRANSPARENT_KEY,
                         highlightthickness=0, bd=0, cursor="arrow")
        self._msg    = "Esperando mensajes…"
        self._tc     = cfg["text_color"]
        self._oc     = cfg["outline_color"]
        self._tgt_tc = self._tc
        self._tgt_oc = self._oc
        self._src_tc = self._tc
        self._src_oc = self._oc
        self._anim_job: str | None = None

        self.bind("<Configure>", lambda _: self.redraw())

    # ── API pública ───────────────────────────────────────────────────────

    def set_message(self, text: str) -> None:
        self._msg = text
        self.redraw()

    def transition_colors(self, tc: str, oc: str, ms: int = 450) -> None:
        """Transición suave al nuevo par de colores en `ms` milisegundos."""
        if self._anim_job:
            self.after_cancel(self._anim_job)
        self._src_tc, self._src_oc = self._tc, self._oc
        self._tgt_tc, self._tgt_oc = tc, oc
        steps = max(ms // 40, 1)
        self._animate(steps, 0)

    def apply_colors_instant(self, tc: str, oc: str) -> None:
        if self._anim_job:
            self.after_cancel(self._anim_job)
        self._tc, self._oc = tc, oc
        self._tgt_tc, self._tgt_oc = tc, oc
        self.redraw()

    def redraw(self) -> None:
        """Repintado completo — llamar cuando cambia cfg o el mensaje."""
        self.delete("all")
        w   = self.winfo_width()  or cfg["overlay_w"]
        h   = self.winfo_height() or max(cfg["overlay_h"] - FRAME_H, 20)
        pad = cfg["badge_padding"]
        r   = cfg["badge_radius"]
        style = cfg["bg_style"]

        # ── Badge de fondo ────────────────────────────────────────────────
        if style in ("badge", "auto_badge"):
            fill    = cfg["badge_color"]
            stipple = "gray50" if style == "auto_badge" else ""
            self._draw_rrect(pad // 2, pad // 2,
                             w - pad // 2, h - pad // 2,
                             r, fill, stipple)

        # ── Texto con contorno ────────────────────────────────────────────
        weight = "bold" if cfg["font_bold"] else "normal"
        fspec  = (cfg["font_family"], cfg["font_size"], weight)
        cx, cy = w // 2, h // 2
        ow     = max(cfg["outline_width"], 0)

        for dx, dy in self._OFFSETS:
            self.create_text(cx + dx * ow, cy + dy * ow,
                             text=self._msg, fill=self._oc,
                             font=fspec, width=w - pad * 2,
                             justify="center", anchor="center")

        self.create_text(cx, cy,
                         text=self._msg, fill=self._tc,
                         font=fspec, width=w - pad * 2,
                         justify="center", anchor="center")

    # ── Internos ──────────────────────────────────────────────────────────

    def _animate(self, total: int, step: int) -> None:
        t = min((step + 1) / total, 1.0)
        t_ease = 1 - (1 - t) ** 3          # ease-out cúbico
        self._tc = lerp_hex(self._src_tc, self._tgt_tc, t_ease)
        self._oc = lerp_hex(self._src_oc, self._tgt_oc, t_ease)
        self.redraw()
        if t < 1.0:
            self._anim_job = self.after(40, self._animate, total, step + 1)

    def _draw_rrect(self, x0: int, y0: int, x1: int, y1: int,
                    r: int, fill: str, stipple: str = "") -> None:
        """
        Rectángulo redondeado usando create_rectangle + create_arc.
        NO usa smooth=True (polígono B-spline) porque los píxeles de
        antialiasing en los bordes no coinciden exactamente con el color
        transparente y aparecen como manchas de color en el overlay.
        """
        r = max(0, min(r, (x1 - x0) // 2, (y1 - y0) // 2))
        kw: dict = {"fill": fill, "outline": ""}
        if stipple:
            kw["stipple"] = stipple

        if r == 0:
            self.create_rectangle(x0, y0, x1, y1, **kw)
            return

        # Cuerpo central horizontal y vertical
        self.create_rectangle(x0 + r, y0,     x1 - r, y1,     **kw)
        self.create_rectangle(x0,     y0 + r, x0 + r, y1 - r, **kw)
        self.create_rectangle(x1 - r, y0 + r, x1,     y1 - r, **kw)

        # Cuatro esquinas redondeadas
        self.create_arc(x0,         y0,         x0 + 2*r, y0 + 2*r,
                        start=90,  extent=90, style="pieslice", **kw)
        self.create_arc(x1 - 2*r,  y0,         x1,       y0 + 2*r,
                        start=0,   extent=90, style="pieslice", **kw)
        self.create_arc(x0,         y1 - 2*r,  x0 + 2*r, y1,
                        start=180, extent=90, style="pieslice", **kw)
        self.create_arc(x1 - 2*r,  y1 - 2*r,  x1,       y1,
                        start=270, extent=90, style="pieslice", **kw)


# ── Escuchador WebSocket (hilo daemon) ────────────────────────────────────
def _ws_listener(url: str, q: queue.Queue) -> None:
    async def _run() -> None:
        delay = 2
        while True:
            try:
                q.put(f"Conectando a {url}…")
                async with websockets.connect(
                    url, ping_interval=20, ping_timeout=20, close_timeout=5
                ) as ws:
                    delay = 2
                    q.put("Conectado ✓")
                    async for raw in ws:
                        try:
                            p = json.loads(raw)
                            if p.get("type") == "message":
                                q.put(f"*{p.get('username', '?')}*: {p.get('content', '')}")
                        except json.JSONDecodeError:
                            pass
            except (websockets.ConnectionClosed, OSError) as e:
                q.put(f"Desconectado. Reintentando en {delay}s…")
                print(f"[ws] {e} — reintento en {delay}s")
            except Exception as e:
                q.put(f"Error: {e}")
                print(f"[ws] {e}")
            await asyncio.sleep(delay)
            delay = min(delay * 2, 60)

    asyncio.run(_run())


# ── Diálogo de ajustes ────────────────────────────────────────────────────
class SettingsDialog(tk.Toplevel):
    """
    Ventana de configuración con pestañas.
    Los cambios se aplican al overlay al pulsar "Aplicar y cerrar";
    "Cancelar" descarta los cambios sin guardar.
    """
    DARK  = "#1E1F22"
    MID   = "#2B2D31"
    ENTRY = "#383A40"
    FG    = "#DCDDDE"
    ACC   = "#5865F2"
    FONTS = ["Arial", "Calibri", "Consolas", "Georgia",
             "Helvetica", "Segoe UI", "Tahoma", "Trebuchet MS", "Verdana"]

    def __init__(self, parent: "OverlayWindow") -> None:
        super().__init__(parent.root)
        self.overlay = parent
        self._orig = dict(cfg)          # instantánea para Cancelar

        self.title("LawfulOverlay — Configuración")
        self.geometry("520x480")
        self.resizable(False, False)
        self.attributes("-topmost", True)
        self.configure(bg=self.DARK)

        # Estilo oscuro para widgets ttk
        style = ttk.Style(self)
        style.theme_use("clam")
        for w in ("TNotebook", "TNotebook.Tab", "TFrame"):
            style.configure(w, background=self.DARK, foreground=self.FG)
        style.configure("TNotebook.Tab",
                        background=self.MID, foreground=self.FG,
                        padding=[10, 4], font=("Arial", 9, "bold"))
        style.map("TNotebook.Tab",
                  background=[("selected", self.ACC)],
                  foreground=[("selected", "#FFFFFF")])

        nb = ttk.Notebook(self)

        # ── Barra inferior — se empaqueta ANTES que el notebook ───────────
        # Tkinter asigna espacio en orden de empaquetado; si el notebook se
        # empaqueta primero con expand=True consume toda la ventana y la barra
        # queda oculta fuera de los límites.
        bar = tk.Frame(self, bg=self.DARK)
        bar.pack(side="bottom", fill="x", padx=10, pady=(0, 10))

        # Fila de botones (derecha)
        btn_row = tk.Frame(bar, bg=self.DARK)
        btn_row.pack(fill="x")

        tk.Button(btn_row, text="Aplicar y cerrar", command=self._aplicar,
                  bg=self.ACC, fg="#FFFFFF", relief="flat",
                  font=("Arial", 9, "bold"), padx=12, pady=4).pack(side="right")

        tk.Button(btn_row, text="Cancelar", command=self._cancelar,
                  bg=self.MID, fg=self.FG, relief="flat",
                  font=("Arial", 9), padx=12, pady=4).pack(side="right", padx=10)

        # Fila de ruta del archivo (abajo, independiente)
        tk.Label(bar, text=f"Archivo: {_cfg_path()}",
                 bg=self.DARK, fg="#666", font=("Consolas", 8),
                 wraplength=500, justify="left"
                 ).pack(side="left", pady=(5, 0))

        # Ahora el notebook ocupa el espacio restante
        nb.pack(fill="both", expand=True, padx=10, pady=(10, 4))

        # ── Variables vinculadas al cfg actual ────────────────────────────
        self.v_url      = tk.StringVar(value=cfg["ws_url"])
        self.v_ow       = tk.IntVar(value=cfg["overlay_w"])
        self.v_oh       = tk.IntVar(value=cfg["overlay_h"])
        self.v_top      = tk.BooleanVar(value=cfg["always_on_top"])
        self.v_family   = tk.StringVar(value=cfg["font_family"])
        self.v_size     = tk.IntVar(value=cfg["font_size"])
        self.v_bold     = tk.BooleanVar(value=cfg["font_bold"])
        self.v_style    = tk.StringVar(value=cfg["bg_style"])
        self.v_badge_c  = tk.StringVar(value=cfg["badge_color"])
        self.v_pad      = tk.IntVar(value=cfg["badge_padding"])
        self.v_radius   = tk.IntVar(value=cfg["badge_radius"])
        self.v_tc       = tk.StringVar(value=cfg["text_color"])
        self.v_oc_color = tk.StringVar(value=cfg["outline_color"])
        self.v_ow2      = tk.IntVar(value=cfg["outline_width"])
        self.v_detect   = tk.BooleanVar(value=cfg["auto_detect"])
        self.v_interval = tk.IntVar(value=cfg["detect_interval"])
        self.v_sample   = tk.IntVar(value=cfg["detect_sample_px"])
        self.v_thresh   = tk.IntVar(value=cfg["detect_threshold"])

        nb.add(self._tab_conexion(nb),  text=" Conexión ")
        nb.add(self._tab_pantalla(nb),  text=" Pantalla ")
        nb.add(self._tab_estilo(nb),    text=" Estilo ")
        nb.add(self._tab_deteccion(nb), text=" Detección ")

    # ── Pestañas ──────────────────────────────────────────────────────────

    def _tab_conexion(self, nb: ttk.Notebook) -> tk.Frame:
        f = self._frame(nb)
        self._row(f, "URL del servidor WebSocket", self.v_url, entry=True, width=34)
        self._sep(f)
        tk.Label(f, text=(
            "El overlay se conecta a este servidor WebSocket.\n"
            "Los cambios se aplican al reiniciar la aplicación."
        ), bg=self.DARK, fg="#888", font=("Arial", 9), justify="left",
        ).pack(anchor="w", padx=16, pady=4)
        return f

    def _tab_pantalla(self, nb: ttk.Notebook) -> tk.Frame:
        f = self._frame(nb)
        self._row(f, "Ancho (px)",   self.v_ow,   spin=(200, 1920))
        self._row(f, "Alto (px)",    self.v_oh,   spin=(60,  400))
        self._check(f, "Siempre encima", self.v_top)
        self._sep(f)
        self._header(f, "Fuente")
        self._combo(f, "Familia", self.v_family, self.FONTS)
        self._row(f, "Tamaño (pt)",  self.v_size, spin=(8, 72))
        self._check(f, "Negrita", self.v_bold)
        return f

    def _tab_estilo(self, nb: ttk.Notebook) -> tk.Frame:
        f = self._frame(nb)
        self._header(f, "Fondo del texto")

        estilos = [
            ("Ninguno (transparente)",             "none"),
            ("Badge sólido",                       "badge"),
            ("Badge adaptativo (semitransparente)", "auto_badge"),
        ]
        for etiqueta, val in estilos:
            rb = tk.Radiobutton(f, text=etiqueta, variable=self.v_style, value=val,
                                bg=self.DARK, fg=self.FG, selectcolor=self.MID,
                                activebackground=self.DARK, activeforeground=self.FG,
                                font=("Arial", 10))
            rb.pack(anchor="w", padx=16, pady=1)

        self._sep(f)
        self._colorpick(f, "Color del badge",          self.v_badge_c)
        self._row(f,       "Relleno del badge (px)",   self.v_pad,     spin=(0, 40))
        self._row(f,       "Radio de esquinas (px)",   self.v_radius,  spin=(0, 30))
        self._sep(f)
        self._colorpick(f, "Color del texto (manual)", self.v_tc)
        self._colorpick(f, "Color del contorno",       self.v_oc_color)
        self._row(f,       "Ancho del contorno (px)",  self.v_ow2,     spin=(0, 8))
        return f

    def _tab_deteccion(self, nb: ttk.Notebook) -> tk.Frame:
        f = self._frame(nb)
        self._check(f, "Activar detección automática de color", self.v_detect)
        self._sep(f)
        self._row(f, "Intervalo (ms)",          self.v_interval, spin=(500, 30_000))
        self._row(f, "Tamaño de muestra (px)",  self.v_sample,   spin=(20,  200))
        self._row(f, "Umbral de cambio",         self.v_thresh,   spin=(5,   100))
        self._sep(f)
        tk.Label(f, text=(
            "La detección lee un pequeño cuadrado de píxeles (tamaño de muestra\n"
            "× tamaño de muestra) situado en el centro del área de contenido,\n"
            "solo al intervalo indicado.  Ningún dato se almacena ni se envía.\n\n"
            "Consejo: el estilo 'Badge adaptativo' queda genial con la detección activa."
        ), bg=self.DARK, fg="#888", font=("Arial", 9), justify="left",
        ).pack(anchor="w", padx=16, pady=4)
        return f

    # ── Constructores de controles ────────────────────────────────────────

    def _frame(self, nb: ttk.Notebook) -> tk.Frame:
        f = tk.Frame(nb, bg=self.DARK)
        f.pack(fill="both", expand=True)
        return f

    def _header(self, parent: tk.Frame, texto: str) -> None:
        tk.Label(parent, text=texto, bg=self.DARK, fg=self.ACC,
                 font=("Arial", 9, "bold")).pack(anchor="w", padx=12, pady=(8, 2))

    def _sep(self, parent: tk.Frame) -> None:
        tk.Frame(parent, bg="#3A3B3F", height=1).pack(fill="x", padx=12, pady=6)

    def _row(self, parent: tk.Frame, label: str, var: tk.Variable,
             entry: bool = False, width: int = 8,
             spin: tuple[int, int] | None = None) -> None:
        row = tk.Frame(parent, bg=self.DARK)
        row.pack(fill="x", padx=12, pady=3)
        tk.Label(row, text=label, bg=self.DARK, fg=self.FG,
                 font=("Arial", 10), width=24, anchor="w").pack(side="left")
        if entry:
            tk.Entry(row, textvariable=var, bg=self.ENTRY, fg=self.FG,
                     insertbackground=self.FG, relief="flat",
                     font=("Consolas", 10), width=width).pack(side="left", padx=4)
        elif spin:
            tk.Spinbox(row, from_=spin[0], to=spin[1], textvariable=var,
                       bg=self.ENTRY, fg=self.FG, insertbackground=self.FG,
                       relief="flat", font=("Arial", 10), width=7,
                       buttonbackground=self.MID).pack(side="left", padx=4)

    def _check(self, parent: tk.Frame, label: str, var: tk.BooleanVar) -> None:
        tk.Checkbutton(parent, text=label, variable=var,
                       bg=self.DARK, fg=self.FG, selectcolor=self.MID,
                       activebackground=self.DARK, activeforeground=self.FG,
                       font=("Arial", 10)).pack(anchor="w", padx=12, pady=3)

    def _combo(self, parent: tk.Frame, label: str,
               var: tk.StringVar, values: list[str]) -> None:
        row = tk.Frame(parent, bg=self.DARK)
        row.pack(fill="x", padx=12, pady=3)
        tk.Label(row, text=label, bg=self.DARK, fg=self.FG,
                 font=("Arial", 10), width=24, anchor="w").pack(side="left")
        ttk.Combobox(row, textvariable=var, values=values,
                     state="readonly", width=20).pack(side="left", padx=4)

    def _colorpick(self, parent: tk.Frame, label: str, var: tk.StringVar) -> None:
        row = tk.Frame(parent, bg=self.DARK)
        row.pack(fill="x", padx=12, pady=3)
        tk.Label(row, text=label, bg=self.DARK, fg=self.FG,
                 font=("Arial", 10), width=24, anchor="w").pack(side="left")

        preview = tk.Label(row, width=4, relief="flat", bg=var.get())
        preview.pack(side="left", padx=4)

        def elegir() -> None:
            elegido = colorchooser.askcolor(color=var.get(), parent=self, title=label)
            if elegido[1]:
                var.set(elegido[1])
                preview.config(bg=elegido[1])

        tk.Button(row, text="Elegir…", command=elegir,
                  bg=self.MID, fg=self.FG, relief="flat",
                  font=("Arial", 9), padx=8).pack(side="left")

    # ── Aplicar / Cancelar ────────────────────────────────────────────────

    def _aplicar(self) -> None:
        cfg.update({
            "ws_url":           self.v_url.get().strip(),
            "overlay_w":        self.v_ow.get(),
            "overlay_h":        self.v_oh.get(),
            "always_on_top":    self.v_top.get(),
            "font_family":      self.v_family.get(),
            "font_size":        self.v_size.get(),
            "font_bold":        self.v_bold.get(),
            "bg_style":         self.v_style.get(),
            "badge_color":      self.v_badge_c.get(),
            "badge_padding":    self.v_pad.get(),
            "badge_radius":     self.v_radius.get(),
            "text_color":       self.v_tc.get(),
            "outline_color":    self.v_oc_color.get(),
            "outline_width":    self.v_ow2.get(),
            "auto_detect":      self.v_detect.get(),
            "detect_interval":  self.v_interval.get(),
            "detect_sample_px": self.v_sample.get(),
            "detect_threshold": self.v_thresh.get(),
        })
        save_cfg()
        self.overlay.apply_settings()
        self.destroy()

    def _cancelar(self) -> None:
        cfg.update(self._orig)
        self.destroy()


# ── Ventana principal del overlay ──────────────────────────────────────────
class OverlayWindow:
    def __init__(self, root: tk.Tk) -> None:
        self.root = root
        self._last_bg: tuple[int, int, int] | None = None
        self._detect_job: str | None = None

        # ── Configuración de la ventana ───────────────────────────────────
        self.root.title("LawfulOverlay")
        self.root.overrideredirect(True)
        self.root.attributes("-topmost", cfg["always_on_top"])
        self.root.attributes("-transparentcolor", TRANSPARENT_KEY)
        self.root.config(bg=TRANSPARENT_KEY)
        self._apply_geometry()

        # ── Barra de título ───────────────────────────────────────────────
        self.bar = tk.Frame(self.root, bg=CHROME_BG, height=FRAME_H)
        self.bar.pack(fill="x", side="top")
        self.bar.pack_propagate(False)

        self.lbl_titulo = tk.Label(self.bar, text="Discord Overlay",
                                   bg=CHROME_BG, fg=CHROME_FG,
                                   font=("Arial", 9, "bold"))
        self.lbl_titulo.pack(side="left", padx=10)

        btn_cerrar = tk.Button(self.bar, text="×", command=self.cerrar,
                               bg="#ED4245", fg="white", font=("Arial", 11, "bold"),
                               relief="flat", bd=0, width=3, cursor="hand2")
        btn_cerrar.pack(side="right", padx=4, pady=3)

        btn_config = tk.Button(self.bar, text="⚙", command=self._abrir_config,
                               bg="#5865F2", fg="white", font=("Arial", 10, "bold"),
                               relief="flat", bd=0, width=3, cursor="hand2")
        btn_config.pack(side="right", padx=2, pady=3)

        # ── Canvas de contenido ───────────────────────────────────────────
        self.canvas = OverlayCanvas(self.root)
        self.canvas.pack(fill="both", expand=True)

        # ── Arrastre ──────────────────────────────────────────────────────
        self._ox = self._oy = 0
        for w in (self.bar, self.lbl_titulo):
            w.bind("<ButtonPress-1>", self._drag_inicio)
            w.bind("<B1-Motion>",     self._drag_mover)

        # ── Arrancar bucles ───────────────────────────────────────────────
        self._iniciar_ws()
        self._sondear_mensajes()
        self._programar_deteccion()

    # ── WebSocket ─────────────────────────────────────────────────────────

    def _iniciar_ws(self) -> None:
        threading.Thread(
            target=_ws_listener, args=(cfg["ws_url"], _msg_q), daemon=True
        ).start()

    # ── Sondeo de mensajes ────────────────────────────────────────────────

    def _sondear_mensajes(self) -> None:
        try:
            while not _msg_q.empty():
                self.canvas.set_message(_msg_q.get_nowait())
        except queue.Empty:
            pass
        self.root.after(100, self._sondear_mensajes)

    # ── Detección de color de fondo ───────────────────────────────────────

    def _programar_deteccion(self) -> None:
        if self._detect_job:
            self.root.after_cancel(self._detect_job)
        if cfg["auto_detect"]:
            self._detect_job = self.root.after(
                cfg["detect_interval"], self._detectar
            )

    def _detectar(self) -> None:
        """
        Captura un cuadrado de píxeles en el centro del área de contenido
        para determinar el color de fondo y actualizar colores de texto/badge.
        """
        try:
            sx = self.root.winfo_x() + cfg["overlay_w"] // 2 - cfg["detect_sample_px"] // 2
            sy = (self.root.winfo_y() + FRAME_H
                  + (cfg["overlay_h"] - FRAME_H) // 2 - cfg["detect_sample_px"] // 2)
            bg = sample_bg(sx, sy, cfg["detect_sample_px"])

            if bg is not None:
                if self._last_bg is None or any(
                    abs(int(a) - int(b)) > cfg["detect_threshold"]
                    for a, b in zip(bg, self._last_bg)
                ):
                    self._last_bg = bg
                    lum = relative_luminance(*bg)
                    tc, oc = wcag_best_pair(lum)

                    if cfg["bg_style"] == "auto_badge":
                        cfg["badge_color"] = auto_badge_color(lum)

                    self.canvas.transition_colors(tc, oc, ms=450)
        except Exception as e:
            print(f"[detección] {e}")
        finally:
            self._programar_deteccion()

    # ── Arrastre ──────────────────────────────────────────────────────────

    def _drag_inicio(self, event: tk.Event) -> None:
        self._ox = event.x_root - self.root.winfo_x()
        self._oy = event.y_root - self.root.winfo_y()

    def _drag_mover(self, event: tk.Event) -> None:
        self.root.geometry(
            f"+{event.x_root - self._ox}+{event.y_root - self._oy}"
        )

    # ── Configuración ─────────────────────────────────────────────────────

    def _abrir_config(self) -> None:
        SettingsDialog(self)

    def apply_settings(self) -> None:
        """Llamado por SettingsDialog tras Aplicar — propaga cfg al overlay."""
        self._apply_geometry()
        self.root.attributes("-topmost", cfg["always_on_top"])
        if not cfg["auto_detect"]:
            self.canvas.apply_colors_instant(
                cfg["text_color"], cfg["outline_color"]
            )
        self.canvas.redraw()
        self._programar_deteccion()

    def _apply_geometry(self) -> None:
        self.root.geometry(f"{cfg['overlay_w']}x{cfg['overlay_h']}")

    # ── Cierre ────────────────────────────────────────────────────────────

    def cerrar(self) -> None:
        print("Cerrando overlay.")
        self.root.destroy()


# ── Punto de entrada ──────────────────────────────────────────────────────
def main() -> None:
    print("LawfulOverlay — Cliente")
    load_cfg()
    print(f"Servidor: {cfg['ws_url']}")

    root = tk.Tk()
    app = OverlayWindow(root)
    root.protocol("WM_DELETE_WINDOW", app.cerrar)
    root.mainloop()


if __name__ == "__main__":
    main()