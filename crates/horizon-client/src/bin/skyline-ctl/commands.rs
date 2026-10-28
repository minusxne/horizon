//! Every command skyline-ctl knows: its usage, help, examples and arguments.
//! The same table prints `help`, checks `COMMAND --help` and writes the
//! generated reference (`help --markdown`).

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    Int,
    Num,
    Str,
    Bool,
    /// One of these words.
    Enum(&'static [&'static str]),
    /// The rest of the line, spaces included.
    Rest,
}

#[derive(Clone, Copy)]
pub struct Param {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
    /// Sent as `--name value` (`--name` alone for booleans), not by position.
    pub flag: bool,
    pub help: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Session,
    Observe,
    Move,
    Act,
    Screens,
    Multiplayer,
    World,
    Scripting,
}
impl Group {
    pub const ALL: [Group; 8] = [
        Group::Session,
        Group::Observe,
        Group::Move,
        Group::Act,
        Group::Screens,
        Group::Multiplayer,
        Group::World,
        Group::Scripting,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Group::Session => "Session",
            Group::Observe => "Observe (JSON state)",
            Group::Move => "Move and look",
            Group::Act => "Act on the world",
            Group::Screens => "Screens, menus and chat",
            Group::Multiplayer => "Multiplayer and LAN",
            Group::World => "World and cheats",
            Group::Scripting => "Scripting and benchmarks",
        }
    }
}

pub struct Cmd {
    pub name: &'static str,
    /// What the game receives instead of `name` (aliases such as look-at-block).
    pub wire: Option<&'static str>,
    pub group: Group,
    pub usage: &'static str,
    pub summary: &'static str,
    pub details: &'static str,
    pub examples: &'static [&'static str],
    pub params: &'static [Param],
    /// Runs in skyline-ctl itself, not in the game.
    pub local: bool,
}

const fn p(name: &'static str, ty: Ty, help: &'static str) -> Param {
    Param {
        name,
        ty,
        required: true,
        flag: false,
        help,
    }
}
const fn opt(name: &'static str, ty: Ty, help: &'static str) -> Param {
    Param {
        name,
        ty,
        required: false,
        flag: false,
        help,
    }
}
const fn flag(name: &'static str, ty: Ty, help: &'static str) -> Param {
    Param {
        name,
        ty,
        required: false,
        flag: true,
        help,
    }
}
const XYZ: [Param; 3] = [
    p("x", Ty::Num, "X coordinate"),
    p("y", Ty::Num, "Y coordinate"),
    p("z", Ty::Num, "Z coordinate"),
];
const BLOCK_XYZ: [Param; 3] = [
    p("x", Ty::Int, "block X"),
    p("y", Ty::Int, "block Y"),
    p("z", Ty::Int, "block Z"),
];
const OVER: Param = flag(
    "over",
    Ty::Num,
    "turn with mouse motion spread over this many seconds (eased), as a player would",
);
const FACE: Ty = Ty::Enum(&["down", "up", "north", "south", "west", "east"]);
const BOX: [Param; 6] = [
    p("x1", Ty::Int, "corner X"),
    p("y1", Ty::Int, "corner Y"),
    p("z1", Ty::Int, "corner Z"),
    p("x2", Ty::Int, "opposite X"),
    p("y2", Ty::Int, "opposite Y"),
    p("z2", Ty::Int, "opposite Z"),
];
const PAGES: Ty = Ty::Enum(&[
    "main",
    "video",
    "sound",
    "controls",
    "keybinds",
    "controller",
    "controller-binds",
    "mouse",
    "chat",
    "accessibility",
    "language",
    "font",
    "packs",
    "mods",
    "shader-packs",
    "graphics",
    "graphics-advanced",
    "extras",
    "horizon",
]);

/// `--confirm`, required by destructive commands: the player agreed.
const CONFIRM: Param = Param {
    name: "confirm",
    ty: Ty::Bool,
    required: true,
    flag: true,
    help: "true once the player agreed: it cannot be undone",
};

macro_rules! cmd {
    ($name:expr, $group:ident, $usage:expr, $summary:expr, $details:expr, [$($ex:expr),* $(,)?], $params:expr, local: $local:expr $(, wire: $wire:expr)?) => {
        Cmd {
            name: $name,
            wire: cmd!(@wire $($wire)?),
            group: Group::$group,
            usage: $usage,
            summary: $summary,
            details: $details,
            examples: &[$($ex),*],
            params: $params,
            local: $local,
        }
    };
    (@wire) => { None };
    (@wire $w:expr) => { Some($w) };
}

pub const COMMANDS: &[Cmd] = &[
    // Session
    cmd!("launch", Session, "launch [--world NAME] [--seed N] [--creative] [--version TAG] [--game PATH] [--wait] [GAME ARGS...]",
        "start Skyline with the control socket and wait until it answers",
        "Finds the game in this order: --game PATH; a horizon-client next to skyline-ctl (a build folder, a release folder or a launcher install); the newest version installed by the Skyline launcher (--version TAG picks one). Launcher installs run with their prepared resources and keep saves in the game home ($SKYLINE_HOME, else ~/.local/share/skyline). --wait also waits until the player has spawned and chunks are loaded. Other arguments go to the game (horizon-client --help lists them). The game's output goes to a .log file beside the socket.",
        ["skyline-ctl launch --world Survival --seed 42 --wait", "skyline-ctl launch --world Flat --creative --render-distance 8"],
        &[flag("world", Ty::Str, "world folder to open or create"), flag("seed", Ty::Str, "seed for a new world"), flag("creative", Ty::Bool, "creative mode for a new world"),
          flag("version", Ty::Str, "an installed launcher version tag such as v2026.10.13"), flag("game", Ty::Str, "path to a horizon-client binary"),
          flag("wait", Ty::Bool, "also wait until the world is ready"),
          flag("offscreen", Ty::Bool, "render without showing a window (screenshots still work): locked or headless sessions")],
        local: true),
    cmd!("wait-for", Session, "wait-for screen NAME|widget ID|loaded|chat TEXT|event KIND [--timeout S] [--since SEQ]", "wait until the game reaches a state, then reply",
        "screen takes the names state shows (playing, inventory, paused, dead, title...); widget an id from ui; loaded means a world is open, the player is in it and no chunk is pending; chat a new line containing TEXT; event a kind from events (death, respawn, item, screen...). chat and event count from --since SEQ (take last from events before acting), else from now. A timeout (default 30 s) is an error with kind timeout.",
        ["skyline-ctl wait-for screen inventory", "skyline-ctl wait-for loaded --timeout 120", "skyline-ctl wait-for chat joined the game", "skyline-ctl wait-for event death --timeout 600"],
        &[flag("timeout", Ty::Num, "seconds (default 30)"), flag("since", Ty::Int, "chat and event: count from this event seq"), p("condition", Ty::Enum(&["screen", "widget", "loaded", "chat", "event"]), "what to wait for"), opt("value", Ty::Rest, "the screen name, widget id, chat text or event kind")], local: false),
    cmd!("events", Observe, "events [--since SEQ]", "what happened since SEQ, oldest first: screens, chat, deaths, respawns, items gained or lost, dimensions, worlds",
        "Every event has seq and tick; pass the reply's last as --since next time. The game keeps the newest 256.",
        ["skyline-ctl events", "skyline-ctl events --since 42"], &[flag("since", Ty::Int, "only events after this seq")], local: false),
    cmd!("wait-ready", Session, "wait-ready [SECONDS]", "wait until the player has spawned and no chunks are pending",
        "Polls status every 250 ms; replies with the status, or an error after SECONDS (default 120).",
        ["skyline-ctl wait-ready 300"], &[opt("seconds", Ty::Num, "how long to wait (default 120)")], local: true),
    cmd!("logs", Session, "logs [--lines N] [--crash]", "the end of the game's log, or its newest crash report",
        "Reads the game home ($SKYLINE_HOME, else ~/.local/share/skyline): logs/latest.log (the last N lines, default 100) or, with --crash, the newest file in crash-reports. Read-only; works whether the game runs or not.",
        ["skyline-ctl logs", "skyline-ctl logs --lines 300", "skyline-ctl logs --crash"],
        &[flag("lines", Ty::Int, "how many lines of latest.log (default 100)"), flag("crash", Ty::Bool, "the newest crash report instead")],
        local: true),
    cmd!("status", Session, "status", "game, renderer and streaming state (the scripting socket's original summary)",
        "Also what F3 shows: the overlay flags, the renderer, chunk streaming and memory.", ["skyline-ctl status"], &[], local: false),
    cmd!("worlds", Session, "worlds", "the saved worlds, newest first: folder, name, mode, last played, version",
        "folder is what open, world-rename, world-backup and world-delete take.", ["skyline-ctl worlds"], &[], local: false),
    cmd!("world-rename", Session, "world-rename FOLDER NAME", "rename a world, as Edit World does (its folder stays)",
        "Needs the title screen (no world open).", ["skyline-ctl world-rename Test \"My Base\""],
        &[p("folder", Ty::Str, "world folder from worlds"), p("name", Ty::Rest, "the new name")], local: false),
    cmd!("world-backup", Session, "world-backup FOLDER", "zip a world into backups/, as Edit World's Make Backup does",
        "Needs the title screen. Replies once the zip is written, with its size.", ["skyline-ctl world-backup Test"],
        &[p("folder", Ty::Str, "world folder from worlds")], local: false),
    cmd!("world-delete", Session, "world-delete FOLDER --confirm", "delete a world for good, as Select World's Delete does",
        "Needs the title screen and --confirm: ask the player first, it cannot be undone (world-backup keeps a copy).",
        ["skyline-ctl world-delete Test --confirm"],
        &[p("folder", Ty::Str, "world folder from worlds"), CONFIRM], local: false),
    cmd!("open", Session, "open WORLD [SEED] [creative]", "open or create a world from the title screen",
        "Opens saves/WORLD, creating it with SEED (and creative mode) when it doesn't exist.",
        ["skyline-ctl open Test 12345", "skyline-ctl open Build 0 creative"],
        &[p("world", Ty::Str, "world folder"), opt("seed", Ty::Int, "seed for a new world"), opt("mode", Ty::Enum(&["creative"]), "creative for a creative world")], local: false),
    cmd!("save", Session, "save", "save the world", "", ["skyline-ctl save"], &[], local: false),
    cmd!("quit", Session, "quit", "save and exit the game", "", ["skyline-ctl quit"], &[], local: false),
    cmd!("screenshot", Session, "screenshot PATH", "write the next rendered frame as a PNG",
        "Relative paths are taken from skyline-ctl's directory.",
        ["skyline-ctl screenshot view.png"], &[], local: false),
    // Observe
    cmd!("state", Observe, "state", "player, world, screen and crosshair target in one reply",
        "position, block_pos, eye, yaw, pitch, velocity, on_ground, health, food, saturation, xp, effects, gamemode, dimension, time, weather, the screen or title menu, the held item, what the crosshair points at and whether an action is running.",
        ["skyline-ctl state"], &[], local: false),
    cmd!("looking-at", Observe, "looking-at", "the block or entity under the crosshair",
        "kind is block, entity or none; a block has pos, state, face, point and distance; an entity has id, type, health and distance.",
        ["skyline-ctl looking-at"], &[], local: false),
    cmd!("entities", Observe, "entities [RADIUS] [TYPE]", "mobs, dropped items and experience orbs near the player, nearest first",
        "RADIUS defaults to 32. TYPE filters by entity type (pig, minecraft:item, ...). Mobs have an id for look-at entity and attack.",
        ["skyline-ctl entities", "skyline-ctl entities 16 cow", "skyline-ctl entities 8 item"],
        &[opt("radius", Ty::Num, "search radius in blocks (default 32)"), opt("type", Ty::Str, "entity type filter")], local: false),
    cmd!("block", Observe, "block X Y Z", "the block state at a position", "",
        ["skyline-ctl block 10 64 -3"], &BLOCK_XYZ, local: false),
    cmd!("light", Observe, "light X1 Y1 Z1 X2 Y2 Z2", "the stored sky and block light of a box",
        "At most 32768 blocks per request. One hex digit per block in `sky` and `block`, x fastest, then z, then y; `.` where no chunk is loaded.",
        ["skyline-ctl light 0 64 0 15 70 15"],
        &BOX, local: false),
    cmd!("blocks", Observe, "blocks X1 Y1 Z1 X2 Y2 Z2", "the non-air blocks of a box, with counts per block",
        "At most 32768 blocks per request.", ["skyline-ctl blocks 0 60 0 15 70 15"],
        &BOX, local: false),
    cmd!("find-block", Observe, "find-block BLOCK [RADIUS]", "the nearest loaded blocks of a kind",
        "Searches a cube of RADIUS (default 32, at most 64) around the player; returns the count and the 16 nearest.",
        ["skyline-ctl find-block oak_log", "skyline-ctl find-block diamond_ore 48"],
        &[p("block", Ty::Str, "block id such as oak_log"), opt("radius", Ty::Int, "search radius (default 32)")], local: false),
    cmd!("inventory", Observe, "inventory", "hotbar, main inventory, armor, offhand, carried stack and item counts",
        "Slots carry the ids click-slot takes: 0-8 hotbar, 9-35 inventory, 500-503 armor (head to feet), 504 offhand. Stacks with components list them as SNBT.",
        ["skyline-ctl inventory"], &[], local: false),
    cmd!("screen", Observe, "screen", "the open screen or title menu, its buttons and its slots",
        "buttons are what click-button takes; slots are what click-slot takes (crafting grid 100-103 in the inventory, 100-108 at a crafting table, result 200, containers from 300, stations from 600). On the title screen, menu has the mode, worlds and the create-world fields.",
        ["skyline-ctl screen"], &[], local: false),
    cmd!("messages", Observe, "messages [N]", "recent chat and system messages, the title and the action bar",
        "N defaults to 20; chat is newest first.", ["skyline-ctl messages 5"], &[opt("n", Ty::Int, "how many chat lines (default 20)")], local: false),
    cmd!("menu", Observe, "menu", "the open menu's slots and carried stack, with station details", "",
        ["skyline-ctl menu"], &[], local: false),
    // Move
    cmd!("move", Move, "move forward|back|left|right SECONDS [--sprint] [--sneak] [--jump]", "walk in a direction relative to the view for a time",
        "Replies when done with the start and end positions.",
        ["skyline-ctl move forward 2", "skyline-ctl move forward 1.5 --sprint --jump"],
        &[p("direction", Ty::Enum(&["forward", "back", "left", "right"]), "direction relative to the view"), p("seconds", Ty::Num, "how long to walk"),
          flag("sprint", Ty::Bool, "sprint"), flag("sneak", Ty::Bool, "sneak"), flag("jump", Ty::Bool, "keep jumping")], local: false),
    cmd!("goto", Move, "goto X Y Z [--near R] [--sprint] [--direct] [--timeout S]", "walk to a position, finding a path through loaded terrain",
        "A* over standable blocks (steps up one block, drops up to three, avoids lava, swims when it must), then steering with jumps when stuck. Without a path it fails at once (--direct walks straight at the target anyway, falls included). --near R stops within R blocks (use 1.5-3 to reach a block you then break). Replies with arrived, position and distance, or an error when stuck or out of time (default 60 s).",
        ["skyline-ctl goto 120 70 -40", "skyline-ctl goto 8 64 3 --near 2 --sprint"],
        &[XYZ[0], XYZ[1], XYZ[2], flag("near", Ty::Num, "arrive within this many blocks (default 0.6)"), flag("sprint", Ty::Bool, "sprint"), flag("direct", Ty::Bool, "walk straight when no path is found"), flag("timeout", Ty::Num, "give up after this many seconds (default 60)")], local: false),
    cmd!("jump", Move, "jump", "jump once and reply when landed", "", ["skyline-ctl jump"], &[], local: false),
    cmd!("look", Move, "look YAW PITCH", "set the view direction in degrees",
        "Yaw 0 faces south (+Z), 90 west (-X), 180 north, -90 east; pitch 90 looks straight down.",
        ["skyline-ctl look 90 10"], &[p("yaw", Ty::Num, "yaw in degrees"), p("pitch", Ty::Num, "pitch in degrees (-90 up .. 90 down)")], local: false),
    cmd!("turn", Move, "turn DYAW DPITCH [--over S]", "rotate the view by an amount",
        "Without --over the view jumps. With --over S the turn is mouse motion fed over S seconds with ease-in-out, through the same path as a real mouse (sensitivity, inverted axes, cinematic camera), and the reply comes once it is done.",
        ["skyline-ctl turn 45 0", "skyline-ctl turn -90 10 --over 0.5"],
        &[p("dyaw", Ty::Num, "yaw change in degrees"), p("dpitch", Ty::Num, "pitch change in degrees"), OVER], local: false),
    cmd!("mouse-move", Move, "mouse-move DX DY", "move the mouse: relative motion that turns the view like a player's mouse",
        "DX and DY are mouse units (pixels of relative motion) added to the frame's accumulated movement and turned by MouseHandler.turnPlayer: degrees = units x 0.15 x (sensitivity x 0.6 + 0.2)^3 x 8 (without the 8 while scoping), inverted axes and the cinematic camera included. Positive DX turns right, positive DY looks down. Works in the world with no screen open; replies with yaw and pitch after the frame that turned.",
        ["skyline-ctl mouse-move 100 0", "skyline-ctl mouse-move -25 40"],
        &[p("dx", Ty::Num, "horizontal motion (positive turns right)"), p("dy", Ty::Num, "vertical motion (positive looks down)")], local: false),
    cmd!("look-at", Move, "look-at X Y Z | look-at block X Y Z [FACE] | look-at entity ID  [--over S]", "turn to face a point, a block (a visible face of it) or an entity",
        "look-at block aims at a face the crosshair can reach and fails when none is visible. --over S turns there with mouse motion over S seconds (the shorter way round) instead of jumping.",
        ["skyline-ctl look-at 10.5 65 3.5", "skyline-ctl look-at block 10 64 3 up", "skyline-ctl look-at entity 42 --over 0.4"],
        &[XYZ[0], XYZ[1], XYZ[2], OVER], local: false),
    cmd!("look-at-block", Move, "look-at-block X Y Z [FACE] [--over S]", "aim the crosshair at a block (same as look-at block)", "",
        ["skyline-ctl look-at-block 10 64 3", "skyline-ctl look-at-block 10 64 3 up --over 0.3"], &[BLOCK_XYZ[0], BLOCK_XYZ[1], BLOCK_XYZ[2], opt("face", FACE, "which face to aim at"), OVER], local: false, wire: "look-at block"),
    cmd!("look-at-entity", Move, "look-at-entity ID [--over S]", "face an entity (same as look-at entity)", "",
        ["skyline-ctl look-at-entity 42"], &[p("id", Ty::Int, "entity id from entities"), OVER], local: false, wire: "look-at entity"),
    cmd!("tp", Move, "tp X Y Z", "teleport the player", "", ["skyline-ctl tp 0 80 0"], &XYZ, local: false),
    // Act
    cmd!("break", Act, "break X Y Z [--timeout S]", "mine a block, holding the attack button until it breaks",
        "Aims at a visible face (the block must be within reach: use goto --near 2 first), mines with the held item, then waits up to a second for the drops. Replies with block_before, block, ticks, drops and picked_up (inventory change).",
        ["skyline-ctl break 10 64 3"], &[BLOCK_XYZ[0], BLOCK_XYZ[1], BLOCK_XYZ[2], flag("timeout", Ty::Num, "give up after this many seconds (default 30)")], local: false),
    cmd!("place", Act, "place X Y Z [--item ITEM]", "place the held item (or ITEM from the hotbar) at an empty position",
        "Clicks the face of a solid neighbour touching X Y Z. Replies with the block placed and the inventory change.",
        ["skyline-ctl place 10 64 4 --item oak_planks"], &[BLOCK_XYZ[0], BLOCK_XYZ[1], BLOCK_XYZ[2], flag("item", Ty::Str, "hotbar item to select first")], local: false),
    cmd!("interact", Act, "interact X Y Z [FACE]", "right-click a block: open a chest or crafting table, press a button, open a door",
        "Replies with the screen afterwards and whether the block changed.", ["skyline-ctl interact 5 64 2"],
        &[BLOCK_XYZ[0], BLOCK_XYZ[1], BLOCK_XYZ[2], opt("face", FACE, "which face to click")], local: false),
    cmd!("use-item", Act, "use-item [--hold SECONDS]", "right-click with the held item: eat, drink, throw, draw a bow",
        "Hold for 1.7 s to eat.", ["skyline-ctl use-item --hold 1.7"], &[flag("hold", Ty::Num, "hold the button this long")], local: false),
    cmd!("attack", Act, "attack [ID]", "swing at what the crosshair targets (after facing entity ID)",
        "Replies with the target's health before and after.", ["skyline-ctl attack", "skyline-ctl attack 42"],
        &[opt("id", Ty::Int, "entity id to face first")], local: false),
    cmd!("select", Act, "select SLOT|ITEM", "select a hotbar slot 1-9, or the hotbar slot holding ITEM", "",
        ["skyline-ctl select 3", "skyline-ctl select oak_planks"], &[p("slot", Ty::Str, "1-9 or an item id")], local: false),
    cmd!("drop", Act, "drop [all]", "drop one of the held item, or the whole stack", "", ["skyline-ctl drop", "skyline-ctl drop all"],
        &[opt("all", Ty::Bool, "drop the whole stack")], local: false),
    cmd!("hold", Act, "hold KEY SECONDS", "press a key, wait, release it", "Keys as for key.", ["skyline-ctl hold w 2"],
        &[p("key", Ty::Str, "key name"), p("seconds", Ty::Num, "how long")], local: true),
    cmd!("key", Act, "key KEY down|up|tap|repeat", "press, release or auto-repeat a key (w a s d space shift ctrl e q esc f3 1-9 arrows ...)",
        "down holds the key until key ... up, release or stop; wait does not release it. move, goto, jump, break and the other actions release the movement keys when they end.",
        ["skyline-ctl key space tap"], &[p("key", Ty::Str, "key name"), p("action", Ty::Enum(&["down", "up", "tap", "repeat"]), "what to do")], local: false),
    cmd!("mouse", Act, "mouse left|right|middle down|up|click", "press or release a mouse button in the world", "",
        ["skyline-ctl mouse right click"], &[p("button", Ty::Enum(&["left", "right", "middle"]), "button"), p("action", Ty::Enum(&["down", "up", "click"]), "what to do")], local: false),
    cmd!("interact-entity", Act, "interact-entity ID", "right-click an entity: trade with a villager, mount a horse, ride a boat, shear a sheep",
        "Faces the entity (from entities), right-clicks it with the held item, waits two ticks and replies with the screen.",
        ["skyline-ctl interact-entity 42"], &[p("id", Ty::Int, "entity id from entities")], local: true),
    cmd!("pad", Act, "pad BUTTON down|up|tap | pad stick left|right X Y | pad trigger left|right V | pad release | pad status", "a virtual controller: buttons, sticks and triggers as a real pad would send them",
        "Buttons by position: south east west north lb rb lt rt select start ls rs up down left right (south is Xbox A / PlayStation cross). Sticks run -1..1 with up positive and stay tilted until centred; triggers 0..1. Merged with any real controller; status lists the connected ones and what is held.",
        ["skyline-ctl pad south tap", "skyline-ctl pad stick left 0 1", "skyline-ctl pad trigger right 1", "skyline-ctl pad status"],
        &[p("what", Ty::Str, "a button name, stick, trigger, release or status"), opt("a", Ty::Str, "down|up|tap, or left|right"), opt("b", Ty::Num, "stick X or trigger value"), opt("c", Ty::Num, "stick Y")], local: false),
    cmd!("release", Act, "release", "release every key and mouse button", "", ["skyline-ctl release"], &[], local: false),
    cmd!("stop", Act, "stop", "cancel the running action (goto, break...) and release every key", "", ["skyline-ctl stop"], &[], local: false),
    // Screens
    cmd!("open-inventory", Screens, "open-inventory", "open the inventory screen (crafting grid 100-103, result 200)", "",
        ["skyline-ctl open-inventory"], &[], local: false),
    cmd!("click-slot", Screens, "click-slot SLOT [right] [shift]", "click a slot of the open inventory or menu",
        "SLOT ids as listed by screen: 0-8 hotbar, 9-35 inventory, 100+ crafting grid, 200 result, 300+ container, 500-503 armor, 504 offhand, 600+ station. A left click picks up or puts down the carried stack; right puts down one item; shift moves the stack across.",
        ["skyline-ctl click-slot 36", "skyline-ctl click-slot 100 right", "skyline-ctl click-slot 200 shift"],
        &[p("slot", Ty::Int, "slot id"), opt("right", Ty::Bool, "right-click"), opt("shift", Ty::Bool, "shift-click")], local: false),
    cmd!("close", Screens, "close", "close the open menu and return to the game", "", ["skyline-ctl close"], &[], local: false),
    cmd!("ui", Screens, "ui", "every widget of the open screen: id, kind, label, bounds, value",
        "Kinds: button, slider (value 0..1), field, row (worlds, servers, packs, languages), tab (selected), slot (slot-ID, with its stack). Ids come from labels (\"FOV: 70\" is fov, a repeat gets -2) or kind-N without one, and stay put when a value changes. Reflects the last drawn frame (frame counts them).",
        ["skyline-ctl ui"], &[], local: false),
    cmd!("click", Screens, "click ID [right|middle] [shift]", "click a widget of ui by its id, as the mouse would",
        "A field takes focus (then type); slot-ID clicks a slot like click-slot. middle presses the middle button (pick block: a creative player clones the stack).",
        ["skyline-ctl click done", "skyline-ctl click video-settings", "skyline-ctl click row-0", "skyline-ctl click slot-0 shift"],
        &[p("id", Ty::Str, "widget id from ui"), opt("right", Ty::Bool, "right-click"), opt("middle", Ty::Bool, "middle-click"), opt("shift", Ty::Bool, "shift-click (slots)")], local: false),
    cmd!("set", Screens, "set ID VALUE", "move a slider of ui to VALUE (0..1), as a click on its track",
        "The option behind it rounds to its own steps; read the new label with ui.",
        ["skyline-ctl set fov 0.5", "skyline-ctl set render-distance 0"],
        &[p("id", Ty::Str, "slider id from ui"), p("value", Ty::Num, "0..1 along the track")], local: false),
    cmd!("options-get", Screens, "options-get [KEY...]", "the game's options (options.txt) by key, with their type and range or choices",
        "Keys are vanilla's: fov, renderDistance, guiScale, gamma, maxFps, lang, resourcePacks, soundCategory_music, key_key.jump... Enums are given and set by name, key bindings by key name (key.keyboard.w).",
        ["skyline-ctl options-get", "skyline-ctl options-get fov renderDistance"], &[opt("keys", Ty::Rest, "option keys (default: all)")], local: false),
    cmd!("options-set", Screens, "options-set KEY VALUE", "set one option, apply it at once and save options.txt, as the options screen does",
        "A value the option refuses is an error saying what it accepts. Lists are JSON: options-set resourcePacks [\"vanilla\"].",
        ["skyline-ctl options-set fov 90", "skyline-ctl options-set renderClouds fast", "skyline-ctl options-set key_key.jump key.keyboard.j"],
        &[p("key", Ty::Str, "option key from options-get"), p("value", Ty::Rest, "the new value")], local: false),
    cmd!("click-button", Screens, "click-button LABEL", "click a button of the open screen or title menu by its label",
        "Case-insensitive; a unique part of the label is enough. screen lists the buttons.",
        ["skyline-ctl click-button Singleplayer", "skyline-ctl click-button \"Create New World\"", "skyline-ctl click-button respawn"],
        &[p("label", Ty::Rest, "button label")], local: false),
    cmd!("click-at", Screens, "click-at X Y [right]", "click at GUI pixel X Y of the window (screen gives button rectangles)", "",
        ["skyline-ctl click-at 200 120"], &[p("x", Ty::Num, "GUI x"), p("y", Ty::Num, "GUI y"), opt("right", Ty::Bool, "right-click")], local: false),
    cmd!("drag-at", Screens, "drag-at [right] X Y X Y...", "press at the first GUI point, move through the others, release at the last", "",
        ["skyline-ctl drag-at 130 100 148 100 166 100", "skyline-ctl drag-at right 130 100 148 100"], &[p("points", Ty::Rest, "right, then GUI x y pairs")], local: false),
    cmd!("scroll-at", Screens, "scroll-at X Y STEPS", "turn the mouse wheel at GUI pixel X Y (positive scrolls up)", "",
        ["skyline-ctl scroll-at 130 100 -1"], &[p("x", Ty::Num, "GUI x"), p("y", Ty::Num, "GUI y"), p("steps", Ty::Num, "wheel steps")], local: false),
    cmd!("type", Screens, "type [--field name|seed] [--replace] TEXT", "type into the focused text field (chat box, world name, anvil, sign...)",
        "On the title's create-world screen, --field name|seed picks the field and --replace clears it first; type \\b deletes one character.",
        ["skyline-ctl type hello everyone", "skyline-ctl type --field name --replace Agent World", "skyline-ctl type --field seed 12345"], &[flag("field", Ty::Enum(&["name", "seed"]), "create-world field to type into"), flag("replace", Ty::Bool, "clear the field first"), p("text", Ty::Rest, "text to type")], local: false),
    cmd!("chat", Screens, "chat TEXT", "send a chat line (or a /command) as if typed", "", ["skyline-ctl chat hello"],
        &[p("text", Ty::Rest, "message")], local: false),
    cmd!("command", Screens, "command TEXT", "run a command (the leading / is optional)", "",
        ["skyline-ctl command time set day", "skyline-ctl command /give @s oak_log 4"], &[p("command", Ty::Rest, "command line")], local: false),
    cmd!("respawn", Screens, "respawn", "respawn from the death screen", "", ["skyline-ctl respawn"], &[], local: false),
    cmd!("pause", Screens, "pause", "open the pause menu", "", ["skyline-ctl pause"], &[], local: false),
    cmd!("resume", Screens, "resume", "leave the pause menu", "", ["skyline-ctl resume"], &[], local: false),
    cmd!("options", Screens, "options [PAGE]", "open the options screen, or one of its pages (Skyline's too)",
        "Then ui lists its widgets and click, set and type change them, as with the mouse. options-set changes an options.txt value without the screen.",
        ["skyline-ctl options video", "skyline-ctl options horizon"], &[opt("page", PAGES, "the page (default: the main options)")], local: false),
    cmd!("book", Screens, "book [next|back|take|done]", "the open book or lectern: its page text, or press one of its buttons",
        "", ["skyline-ctl book", "skyline-ctl book next"], &[opt("action", Ty::Enum(&["next", "back", "take", "done"]), "a button to press")], local: false),
    cmd!("signedit", Screens, "signedit X Y Z [front|back]", "open the sign editor of the sign at X Y Z (then type and close)",
        "", ["skyline-ctl signedit 3 64 1"], &[BLOCK_XYZ[0], BLOCK_XYZ[1], BLOCK_XYZ[2], opt("side", Ty::Enum(&["front", "back"]), "which side")], local: false),
    cmd!("mount-menu", Screens, "mount-menu", "open the inventory of the horse, donkey, mule or llama being ridden",
        "As the inventory key does while riding; also opens the nearest one's.", ["skyline-ctl mount-menu"], &[], local: false),
    cmd!("chat-log", Screens, "chat-log [N]", "the newest N chat messages", "", ["skyline-ctl chat-log 5"], &[], local: false),
    // Multiplayer
    cmd!("connect", Multiplayer, "connect ADDRESS [NAME]", "join a multiplayer server from the title screen",
        "ADDRESS is HOST[:PORT]. Skyline joins with an offline name: online-mode servers refuse it.",
        ["skyline-ctl connect 192.168.1.20", "skyline-ctl connect localhost:25565"],
        &[p("address", Ty::Str, "HOST[:PORT]"), opt("name", Ty::Str, "player name")], local: false),
    cmd!("disconnect", Multiplayer, "disconnect", "leave the server, back to the server list", "", ["skyline-ctl disconnect"], &[], local: false),
    cmd!("servers", Multiplayer, "servers", "the Multiplayer screen's server list: name and address",
        "Add, edit and remove them on the Multiplayer screen (click-button Multiplayer, then ui).", ["skyline-ctl servers"], &[], local: false),
    cmd!("lan", Multiplayer, "lan open [PORT] [commands] [force] | lan close | lan status", "open the world to LAN (Open to LAN), stop, or see the guests",
        "PORT 0 or none picks one; commands lets guests use cheats; force forces the game mode. Only when the player asks.",
        ["skyline-ctl lan open", "skyline-ctl lan status", "skyline-ctl lan close"],
        &[p("action", Ty::Enum(&["open", "close", "status"]), "what to do"), opt("port", Ty::Int, "port for open"), opt("commands", Ty::Bool, "allow cheats"), opt("force", Ty::Bool, "force the game mode")], local: false),
    // World
    cmd!("give", World, "give ITEM [COUNT]", "add items to the inventory", "", ["skyline-ctl give oak_log 16"],
        &[p("item", Ty::Str, "item id"), opt("count", Ty::Int, "how many (default 1)")], local: false),
    cmd!("time", World, "time TICKS", "set the time of day (0 sunrise, 6000 noon, 13000 night)", "", ["skyline-ctl time 6000"],
        &[p("ticks", Ty::Int, "day time")], local: false),
    cmd!("setblock", World, "setblock X Y Z BLOCK[props]", "set a block (with neighbour updates)", "", ["skyline-ctl setblock 0 64 0 stone"],
        &[BLOCK_XYZ[0], BLOCK_XYZ[1], BLOCK_XYZ[2], p("block", Ty::Str, "block state such as oak_stairs[facing=east]")], local: false),
    cmd!("summon", World, "summon KIND [X Y Z] [baby]", "spawn a mob (in front of the player without a position)", "Replies with its id.", ["skyline-ctl summon cow", "skyline-ctl summon pig 10 64 3 baby"],
        &[p("kind", Ty::Str, "mob type"), opt("x", Ty::Num, "X"), opt("y", Ty::Num, "Y"), opt("z", Ty::Num, "Z"), opt("baby", Ty::Bool, "a baby")], local: false),
    cmd!("effect", World, "effect ID SECONDS [AMPLIFIER] | effect clear", "give a status effect, or clear them all", "", ["skyline-ctl effect speed 30 1", "skyline-ctl effect clear"],
        &[p("id", Ty::Str, "effect id such as speed, or clear"), opt("seconds", Ty::Num, "duration"), opt("amplifier", Ty::Int, "level - 1 (default 0)")], local: false),
    cmd!("effects", World, "effects", "the player's active status effects", "", ["skyline-ctl effects"], &[], local: false),
    cmd!("xp", World, "xp LEVELS", "add experience levels (negative removes)", "", ["skyline-ctl xp 30"], &[p("levels", Ty::Int, "levels to add")], local: false),
    cmd!("enchant", World, "enchant ID LEVEL", "enchant the held item (a book stores it)", "", ["skyline-ctl enchant sharpness 5"],
        &[p("id", Ty::Str, "enchantment id"), p("level", Ty::Int, "level")], local: false),
    cmd!("potion", World, "potion ITEM POTION", "give a potion item", "", ["skyline-ctl potion splash_potion healing"],
        &[p("item", Ty::Enum(&["potion", "splash_potion", "lingering_potion", "tipped_arrow"]), "which item"), p("potion", Ty::Str, "potion id")], local: false),
    cmd!("fill", World, "fill X1 Y1 Z1 X2 Y2 Z2 BLOCK", "set a box of loaded blocks", "", ["skyline-ctl fill 0 64 0 4 64 4 glass"],
        &[BOX[0], BOX[1], BOX[2], BOX[3], BOX[4], BOX[5], p("block", Ty::Str, "block state")], local: false),
    cmd!("container", World, "container X Y Z [ITEM COUNT]", "list a container's items, or add items to it", "", ["skyline-ctl container 3 64 1", "skyline-ctl container 3 64 1 diamond 3"],
        &[BLOCK_XYZ[0], BLOCK_XYZ[1], BLOCK_XYZ[2], opt("item", Ty::Str, "item to add"), opt("count", Ty::Int, "how many")], local: false),
    cmd!("render-distance", World, "render-distance N", "chunks, 2..64", "", ["skyline-ctl render-distance 8"], &[p("chunks", Ty::Int, "render distance")], local: false),
    cmd!("wait", World, "wait TICKS", "wait until TICKS game ticks have passed (20 per second)",
        "Keys and mouse buttons held with key ... down or mouse ... down stay held while it waits. Release them with key ... up, release or stop.",
        ["skyline-ctl wait 40", "skyline-ctl key w down && skyline-ctl wait 40 && skyline-ctl key w up"],
        &[p("ticks", Ty::Int, "game ticks")], local: false),
    // Scripting
    cmd!("script", Scripting, "script [FILE] [--keep-going]", "run commands from FILE (or stdin), one per line",
        "Lines are split like a shell (quotes keep spaces); # starts a comment; sleep SECONDS waits. Stops at the first error unless --keep-going. skyline-ctl - is the older form: read stdin and keep going.",
        ["skyline-ctl script play.txt", "printf 'state\\ngoto 0 64 0\\n' | skyline-ctl script"], &[], local: true),
    cmd!("sleep", Scripting, "sleep SECONDS", "wait (in scripts)", "", ["sleep 0.5"], &[], local: true),
    cmd!("bench", Scripting, "bench SECONDS", "reset frame stats, wait, reply with the summary", "", ["skyline-ctl bench 10"], &[p("seconds", Ty::Num, "how long to measure")], local: false),
    cmd!("stats", Scripting, "stats [reset]", "frame-time summary since the last reset", "", ["skyline-ctl stats"], &[opt("reset", Ty::Bool, "start counting again")], local: false),
    cmd!("fly", Scripting, "fly X1 Y1 Z1 X2 Y2 Z2 SECONDS [PITCH [raw]]", "fly the camera along a line, then reply with the frame summary",
        "The reply's tick_ms and tick_ms_max are the world's smoothed tick time over the flight (the server thread's when single player runs on one). With raw, the reply also lists every frame time in milliseconds (frame_ms).",
        ["skyline-ctl fly 0 100 0 500 100 0 20"], &[], local: false),
];

pub fn find(name: &str) -> Option<&'static Cmd> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// `skyline-ctl help`.
pub fn overview() -> String {
    let mut out = String::from(
        "skyline-ctl: play and script a running Skyline from the command line.\n\n\
         usage: skyline-ctl [--socket PATH] [--timeout SECONDS] [--pretty] COMMAND [ARGS...]\n\
         \x20      skyline-ctl COMMAND --help      (or: skyline-ctl help COMMAND)\n",
    );
    for group in Group::ALL {
        out.push_str(&format!("\n{}:\n", group.title()));
        for c in COMMANDS.iter().filter(|c| c.group == group) {
            out.push_str(&format!("  {:<16} {}\n", c.name, c.summary));
        }
    }
    out.push_str(
        "\nEvery reply is one JSON line. Errors are {\"error\": \"...\"}; the exit status is 0 on\n\
         success, 1 when the game replied with an error, 2 for a usage error, 3 when the\n\
         game cannot be reached or started.\n\
         Actions (goto, break, place...) block until done.\n\
         skyline-ctl help --game lists the running game's own commands, test hooks included.\n\
         The socket is $SKYLINE_CONTROL, else $XDG_RUNTIME_DIR/skyline-control.sock (on Windows\n\
         %TEMP%\\horizon-control.json, a localhost TCP port with a token; any path ending in .json\n\
         uses that transport).\n\n\
         First steps:\n\
         \x20 skyline-ctl launch --world Test --seed 1 --wait\n\
         \x20 skyline-ctl state\n\
         \x20 skyline-ctl find-block oak_log\n\
         \x20 skyline-ctl goto X Y Z --near 2 && skyline-ctl break X Y Z\n\
         \x20 skyline-ctl inventory\n",
    );
    out
}

/// `skyline-ctl COMMAND --help`.
pub fn detail(c: &Cmd) -> String {
    let mut out = format!(
        "usage: skyline-ctl {}\n\n{}.\n",
        c.usage,
        capitalize(c.summary)
    );
    if !c.details.is_empty() {
        out.push('\n');
        out.push_str(&wrap(c.details, 78));
    }
    if !c.examples.is_empty() {
        out.push_str("\nExamples:\n");
        for e in c.examples {
            out.push_str(&format!("  {e}\n"));
        }
    }
    out
}

/// The generated reference, docs/skyline-ctl-commands.md (`skyline-ctl help --markdown`).
pub fn markdown() -> String {
    let cell = |s: &str| s.replace('|', "\\|");
    let mut out = String::from(
        "<!-- Generated by `skyline-ctl help --markdown` from the command table in \
         crates/horizon-client/src/bin/skyline-ctl/commands.rs: edit the table, then regenerate. -->\n\
         # skyline-ctl commands\n\n\
         Every command `skyline-ctl` takes and its arguments. The game answers only when it was \
         started with `--control` (`skyline-ctl launch` does that). Positional arguments come \
         first, in order; `--flags` after them. `skyline-ctl help --game` lists the game's own \
         commands, test hooks included.\n",
    );
    for group in Group::ALL {
        out.push_str(&format!(
            "\n## {}\n\n| Command | What it does | Arguments |\n| --- | --- | --- |\n",
            group.title()
        ));
        for c in COMMANDS.iter().filter(|c| c.group == group) {
            let args: Vec<String> = c
                .params
                .iter()
                .map(|p| {
                    let ty = match p.ty {
                        Ty::Int => "integer".to_owned(),
                        Ty::Num => "number".to_owned(),
                        Ty::Str => "string".to_owned(),
                        Ty::Rest => "text".to_owned(),
                        Ty::Bool => "boolean".to_owned(),
                        Ty::Enum(v) => v.join(", "),
                    };
                    let name = if p.flag {
                        format!("--{}", p.name)
                    } else {
                        p.name.to_owned()
                    };
                    let need = if p.required { ", required" } else { "" };
                    format!("`{name}` ({ty}{need}): {}", cell(p.help))
                })
                .collect();
            out.push_str(&format!(
                "| `{}` | {} | {} |\n",
                cell(c.usage),
                cell(&capitalize(c.summary)),
                if args.is_empty() {
                    "none".to_owned()
                } else {
                    args.join("<br>")
                }
            ));
        }
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn wrap(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_documented() {
        let mut seen = std::collections::HashSet::new();
        for c in COMMANDS {
            assert!(seen.insert(c.name), "duplicate command {}", c.name);
            assert!(
                c.usage.starts_with(c.name.split('-').next().unwrap()),
                "{}",
                c.name
            );
            assert!(
                !c.summary.is_empty() && !c.examples.is_empty(),
                "{}",
                c.name
            );
        }
    }

    #[test]
    fn the_generated_reference_is_up_to_date() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/skyline-ctl-commands.md"
        );
        let on_disk = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            on_disk == markdown(),
            "docs/skyline-ctl-commands.md is out of date: run `skyline-ctl help --markdown > docs/skyline-ctl-commands.md`"
        );
        for c in COMMANDS {
            assert!(
                on_disk.contains(&format!("`{}", c.usage.replace('|', "\\|"))),
                "{} undocumented",
                c.name
            );
        }
    }

    #[test]
    fn help_mentions_every_command() {
        let text = overview();
        for c in COMMANDS {
            assert!(text.contains(c.name));
            assert!(detail(c).contains("usage: skyline-ctl"));
        }
    }
}
