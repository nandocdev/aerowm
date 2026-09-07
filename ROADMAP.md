# Roadmap de Desarrollo: AeroWM

Este roadmap está estructurado en **6 sprints secuenciales** organizados bajo una arquitectura incremental: primero se garantiza la lógica matemática pura en memoria, luego el runtime de configuración, el canal de control y finalmente la integración con el subsistema gráfico de Wayland.

---

## Sprint 1: Fundaciones y Geometría del Core (`aerowm-core`)

**Objetivo:** Obtener un crate de Rust puro, sin dependencias gráficas, capaz de calcular distribuciones espaciales de ventanas con cobertura completa de pruebas unitarias.

* [x] **Configuración del Workspace**
  * [x] Inicializar repositorio Git y configurar el `Cargo.toml` raíz con la estructura de workspaces definida.
  * [x] Configurar linters y formateadores (`rustfmt`, `clippy` con flags estrictas de seguridad).
  * [x] Crear el sub-crate `crates/aerowm-core`.

* [x] **Modelado de Primitivas Geométricas**
  * [x] Implementar structs base: `Point`, `Size`, y `Rect` con operaciones de intersección, paddings y márgenes.
  * [x] Crear el identificador semántico `WindowId` basado en enteros únicos autoincrementales.

* [x] **Diseño del Trait `Layout`**
  * [x] Definir la interfaz base `pub trait Layout`: firma que acepta un rectángulo contenedor y un slice de identificadores, retornando un mapa de geometrías.
  * [x] Implementar el layout `Full` (pantalla completa o maximizado).
  * [x] Implementar el layout `MonadTall` (columna principal con pila secundaria parametrizable por ratio).
  * [x] Implementar el layout `Columns` (distribución por columnas dinámicas con ancho relativo).

* [x] **Estructura de `Workspace` y Foco**
  * [x] Implementar el contenedor de estado `Workspace` (gestión de lista ordenada de ventanas, ventana enfocada, pila de historial).
  * [x] Implementar operaciones cardinales de foco: `focus_next()`, `focus_prev()`, `swap_master()`.
  * [x] Escribir suite de pruebas unitarias cubriendo cálculos de layout y transiciones de foco.

---

## Sprint 2: Motor de Configuración y Scripting (`aerowm-lua`)

**Objetivo:** Incrustar el runtime de Luau vía FFI para parsear configuraciones declarativas, mapear layouts y ejecutar callbacks sin comprometer el proceso principal.

* [x] **Integración del Runtime Luau**
  * [x] Crear el sub-crate `crates/aerowm-lua` y agregar la dependencia de `mlua` con soporte nativo de Luau.
  * [x] Implementar sandboxing y límites de memoria base para la máquina virtual de Luau.
  * [x] Configurar el cargador de archivos para buscar por defecto en `~/.config/aerowm/config.luau`.

* [x] **Exposición de Tipos y Bindings**
  * [x] Crear bindings Rust-Luau completos para modificadores de teclado (`Mod1`, `Mod4`, `Shift`, etc.) y atajos (`binds`).
  * [x] Diseñar el modelo tipado en Luau para definición de `workspaces`, atajos (`binds`) y propiedades visuales.
  * [x] Exponer una API imperativa para comandos de ejecución (`aerowm.spawn(...)`).

* [x] **Sistema de Hooks y Manejo de Errores**
  * [x] Diseñar el despachador de eventos (`hooks.rs`) para emitir señales: `window_opened`, `focus_changed`.
  * [x] Implementar manejo seguro de excepciones en evaluación de scripts.
  * [x] Redactar el archivo de tipos de Luau (`aerowm.d.luau`) para autocompletado y tipado estricto en editores de código.

---

## Sprint 3: Protocolo IPC y Herramienta CLI (`aerowm-ipc` & `aerowm-ctl`)

**Objetivo:** Establecer un canal asíncrono de comunicación local por Unix Socket para consultar estado, recibir órdenes en tiempo real y alimentar widgets/barras.

* [x] **Definición del Protocolo IPC**
  * [x] Crear el sub-crate `crates/aerowm-ipc`.
  * [x] Definir el catálogo de mensajes serializables (JSON-RPC) mediante `serde`: comandos de acción y eventos de notificación.

* [x] **Servidor Asíncrono del Compositor**
  * [x] Implementar el bucle de escucha de sockets Unix usando `calloop` (reemplazando tokio para ahorrar memoria) en `aerowm/src/ipc.rs`.
  * [x] Integrar el mecanismo de Pub/Sub: permitir que clientes externos (barras, paneles) se suscriban a eventos de cambio de foco y tags.
  * [x] Crear un buffer de canal (`mpsc`) thread-safe para transferir comandos IPC hacia el hilo principal del compositor (manejado nativamente por calloop y el estado).

* [x] **Implementación de `aerowm-ctl`**
  * [x] Crear el binario en `bin/aerowm-ctl` (movido a `crates/aerowm-ctl`).
  * [x] Implementar comandos CLI básicos con `clap`: `workspace`, `kill`, `reload`.
  * [x] Implementar comando `aerowm-ctl subscribe` para imprimir eventos en streaming continuo (compatible con scripts de `waybar` o `polybar`).

---

## Sprint 4: Compositor Base y Backend Anidado (`aerowm`)

**Objetivo:** Levantar una sesión gráfica anidada dentro de una ventana X11/Wayland existente usando Smithay, procesando ventanas Wayland nativas mediante `xdg-shell`.

* [x] **Inicialización del Compositor con Smithay**
  * [x] Crear el crate ejecutable principal `crates/aerowm`.
  * [x] Inicializar el bucle de eventos (`Calloop`) y estructurar el estado global.
  * [x] Configurar el backend anidado con `winit` (`backend/winit.rs`) e inicializar `GlesRenderer`.

* [x] **Implementación del Protocolo `xdg-shell`**
  * [x] Implementar `XdgShellHandler` de Smithay para capturar solicitudes de nuevas superficies (ventanas).
  * [x] Ligar el ciclo de vida de cada `ToplevelSurface` Wayland con nuestro generador de `WindowId` del Core.
  * [x] Implementar un `DamageTracker` básico y pintar las ventanas en sus `Rect` correspondientes calculados por el motor de Layout.

* [x] **Mapeo de Entradas y Keybindings**
  * [x] Configurar la gestión de asientos (`SeatHandler`) para teclado y cursor.
  * [x] Interceptar las pulsaciones de teclado desde el backend (`WinitEvent::Input`).
  * [x] Delegar el despacho de la combinación (ej. `Super + Enter`) al motor de Luau, y si no hay coincidencia, usar acciones por defecto (`spawn`, `kill_active`, etc).

---

## Sprint 5: Backend Nativo DRM/KMS e Integración de Escritorio

**Objetivo:** Transformar el compositor en un entorno autónomo ejecutable directamente desde la consola (TTY) con soporte de monitores reales y paneles.

* [x] **Backend de Hardware Directo (`udev` / KMS)**
  * [x] Implementar el backend nativo `backend/udev.rs` usando las APIs de DRM, GBM y Libinput de Smithay.
  * [x] Gestionar la detección dinámica de monitores (`OutputHandler`), resoluciones nativas y tasas de refresco (V-Sync).
  * [x] Implementar el paso atómico de cambio de modo de pantalla (*Atomic Mode Setting*).

* [x] **Soporte para Paneles y Barras (`wlr-layer-shell` y `aerowm-bar`)**
  * [x] Crear el binario cliente `aerowm-bar` usando Layer Shell e integrando los widgets definidos en Luau vía IPC.
  * [x] Implementar el protocolo `wlr-layer-shell` en el compositor.
  * [x] Configurar la deducción del área de trabajo (márgenes exclusivos) ocupada por la barra antes de calcular el tiling de las ventanas.

* [x] **Puntero y Foco por Ratón**
  * [x] Implementar manipulación de ventanas con cursor: arrastre y cambio de tamaño interactivo para ventanas flotantes.
  * [x] Integrar lógica de foco: foco al pasar el puntero (*focus-follows-mouse*) con activación de ventana al hacer clic.

---

## Sprint 6: Optimización, Empaquetado y Pulido

**Objetivo:** Maximizar la fluidez visual, garantizar hot reload atómico, añadir compatibilidad legacy y empaquetar para distribuciones Linux.

* [x] **Soporte Opcional para Aplicaciones Legacy (`XWayland`)**
  * [x] Configurar soporte tras un flag de compilación de Cargo (`--features xwayland`) para ahorrar memoria en instalaciones puras Wayland.
  * [x] Inicializar el socket y contexto de XWayland dentro del ciclo de vida del compositor.
  * [x] Integrar ventanas X11 convencionales dentro del árbol de distribución de Smithay y del core de layouts.

* [x] **Persistencia de Estado y Reinicio en Caliente**
  * [x] Serializar estado del WM (workspaces, layouts, ventanas) en `session.json` al solicitar un reinicio.
  * [x] Implementar transferencia del File Descriptor (FD) del socket de Wayland al nuevo binario vía `exec` para sobrevivir reinicios sin matar aplicaciones.

* [x] **Hot Reload Atómico de Configuración**
  * [x] Implementar comando de recarga en caliente (`reload`): reevaluar `config.luau` en una nueva VM aislada sin destruir superficies activas ni reiniciar clientes Wayland.
  * [x] Implementar evaluación declarativa de Reglas de Ventana (`Match`/`Rule`) vía Luau (auto-asignar float, workspace, ocultas en Scratchpad).
  * [x] Intercambio atómico de punteros de configuración en memoria con tiempo de ejecución $< 15 \text{ ms}$.

* [ ] **Protocolos Extendidos de Wayland**
  * [ ] Implementar `xdg-decoration-v1`, `fractional-scale-v1` y `viewporter` para escalado HiDPI y decoraciones SSD/CSD.
  * [ ] Implementar `ext-session-lock-v1` (bloqueo de pantalla) y `wlr-screencopy-v1` (capturas y streaming).
  * [ ] **Optimización de Memoria y Renderizado**
  * [ ] Aplicar renderizado por regiones dañadas (*damaged-based rendering*) para pintar únicamente los rectángulos de la pantalla que sufrieron cambios.

* [ ] **Empaquetado y Distribución**
  * [ ] Crear el descriptor de sesión de escritorio `assets/aerowm.desktop` para compatibilidad con display managers (GDM, SDDM).
  * [ ] Generar recetas de empaquetado para Arch Linux (`PKGBUILD`) y binarios estáticos auto-contenidos.
  * [ ] Crear batería de configuraciones de ejemplo documentadas (`examples/config.luau`).