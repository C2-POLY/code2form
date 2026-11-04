# Projects

An object is more than one file: its designs, the scan of the part it
starts from, its circuit, its firmware, its brief and what it exports. A
project gathers them: a folder, and a `.c2fproj` file in it that says
which file is what. The studio opens it in one go, the agent reads it in
one call, and the app on iPhone saves its scans into it.

## The folder

A new project starts with:

```
Grogu/
  grogu.c2fproj   the project: which file is what
  brief.md        the brief, to fill in
  models/         the designs (.c2f)
  scans/          the scans of the part the object starts from
  circuit/        the circuit (.c2fc)
  firmware/       the firmware's PlatformIO project
  exports/        what the studio exports: STL, 3MF, STEP, glb…
```

The files may live elsewhere in the folder: the project's file names
them. A folder that holds an object already becomes a project as it is
(File › Make a Project of This Folder, or `c2f project adopt`): the
examples that come with the app (Grogu, the watch winder, the night light
and the robot arm) are projects made that way, their files where they
were.

## The `.c2fproj` file

TOML, as a circuit. Its paths are relative to its folder, and none may
leave it (`..` or an absolute path is an error, at its line).

```toml
version = 1
name = "Grogu"
brief = "brief.md"
# The first design is the main one.
designs = [
  "models/grogu_plate.c2f",
]
circuit = "circuit/grogu.c2fc"
firmware = "firmware"
scans = [
  "scans/scan-2026-11-03-101500.stl",
]
exports = "exports"
```

| Key | What it is |
|---|---|
| `version` | The format's version: 1. Required. |
| `name` | The object's name, as the studio's panel and title show it. Required. |
| `brief` | The brief `plan_object` writes, in Markdown. |
| `designs` | The designs, `.c2f` or `.scad`. The first is the main one, which the project opens on. |
| `circuit` | The circuit, `.c2fc`. |
| `firmware` | The firmware's PlatformIO project, a folder. Left out: `firmware/` beside the circuit, as for a circuit outside any project. |
| `scans` | The scans, STL, OBJ, 3MF or PLY. |
| `exports` | Where the studio's exports go. Left out: `exports/`. |

A design or a scan put in `models/` or `scans/` while the studio has the
project open (by the agent, from the iPhone, from the Finder) joins it on
its own; one taken out of the file by hand stays out. A file of
everything (home, Documents, Desktop, Downloads, iCloud Drive) cannot
become a project.

## In the studio

- **File › New Project** asks for the object's name and where it goes
  (`~/Documents/Code2Form/Projects` by default), then creates the folder.
- **File › Open Project** opens a `.c2fproj`. Opening a file of a project
  opens its project too, when no other is open, and the project open
  comes back at the next launch with the tabs (when the settings reopen
  them).
- **Each project has its own tabs**: the tabs at the top are the files
  of the project open. Opening another project, or a file of another,
  puts them away and brings back its own as they were left; a file out
  of any project opens with the other files out of any. Quitting asks
  about the unsaved edits of every project, putting away or not.
- **The panel on the left** shows the project two ways, **Project** and
  **Files**. Project lists the brief, the designs (the
  main one starred), the scans, the circuit, the firmware and the
  exports. Files shows the project's folder as it is on disk, folders
  first, a firmware's build left out: a design, a circuit or a mesh
  opens in its tab, a firmware's source in its circuit's Firmware tab,
  another file in the Finder. Without a project, File › Show the Folder
  shows the folder of the file open. A click opens a file in its tab: a scan to look at and build
  on, the firmware in its circuit's Firmware tab, an export in the
  Finder. The `+` of Designs writes a new part in `models/`, that of
  Scans imports a mesh into `scans/`, that of Circuit writes the circuit
  in `circuit/` with the main design as its enclosure.
- **Exports** of a file of the project go to its `exports/`, and the
  circuit's firmware to the project's `firmware`.
- **The agent** in the studio works in the project's folder.

## From the command line

```bash
c2f project new ~/Documents/Code2Form/Projects/Lamp   # a new project
c2f project info examples/grogu                        # what it holds
c2f project adopt path/to/folder                       # a folder as it is
```

`info` lists the brief, the designs, the circuit, the firmware (and
whether it is written), the scans, the exports and the files the project
names that are missing; it fails on a badly written file. `--name` names
the project in `new` and `adopt`.

## For the agent

`project_info` gives what a project holds from its file, its folder or
any file in it, the circuit checked; `project_new` creates one;
`project_add` names a file written in it. The prompts `plan_object`,
`design_object` and `add_electronics` create the project, then put the
brief, the design and the circuit in it.

## On iPhone

After a scan, **Save to a Project…** asks for a folder: a project, or a
folder of projects (Code2Form › Projects in iCloud Drive, which the
studio on the Mac sees too), to pick one among them (with one only, the
scan goes in it), or a folder without any, to create one in. The scan goes
into the project's `scans/`, named in its file.
