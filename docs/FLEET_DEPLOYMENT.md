# Fleet deployment: setting policy across technician machines

This is for the person who deploys meetily++ to a team and needs the same rules on
every laptop without visiting each one. No code reading required.

## How it works, in one paragraph

On startup the app looks for a single JSON file in a fixed location on the machine.
If it finds one, the settings in it override what the local user chose. If it does
not find one, nothing changes and the app behaves exactly as it always has. Any MDM
or RMM that can drop a file on disk can therefore set policy: Intune, Jamf, a
PowerShell or shell script, a configuration-management tool, or a login script. There
is no enrolment step, no account, and no server of ours involved, which also means
this feature adds no network traffic of any kind.

## Where the file goes

| Platform | Path |
| --- | --- |
| macOS | `/Library/Application Support/meetily++/managed-config.json` |
| Windows | `%ProgramData%\meetily++\managed-config.json` |
| Linux | `/etc/meetily++/managed-config.json` |

These are all machine-wide locations that need administrator rights to write, which
is the point: a technician should not be able to edit their own policy. Create the
`meetily++` folder if it is not there.

The exact path the app is looking at is shown in the app itself, so you can confirm
you got it right: **Settings → Consent** and **Settings → Privacy profiles** show a
banner naming the file when one is found, and what could not be read when one is
malformed.

## When it takes effect

At the next launch. If you have just pushed a change and do not want to make someone
restart, the app can re-read the file on demand (it is the same action as restarting,
without the restart). Every launch also writes one line into the app's own consent
log recording which policy applied, so "what was the rule on this machine last
Tuesday" is an answerable question after the fact.

## The rules, before the key list

Three things are worth knowing before you write a file, because they decide what
your policy actually does:

1. **Managed settings are bounds, not values.** Setting a minimum consent level of
   "notify" means nobody can go below notify. A technician who wants to confirm every
   speaker individually still can. The same applies to retention: setting 90 days
   means "no longer than 90", and a technician who sets 30 keeps 30.

2. **`locked` removes the local half.** Naming a key in the `locked` list makes the
   managed value the exact value, and greys out the matching control in the app with
   a visible "managed by your organisation" note. This is the only way a policy can
   make a machine *less* strict than the user asked for, so lock deliberately.

3. **Anything the app does not understand is ignored, loudly.** A misspelled value, a
   number where a list belongs, a key from a future version: each is skipped and
   reported in the app's banner rather than guessed at. A policy file is never
   silently interpreted as something weaker than you wrote. A file that is not valid
   JSON at all is rejected whole, and the machine falls back to local settings with
   the reason on screen.

## Key reference

Every key is optional. Leaving one out means "the organisation has no opinion; the
local setting governs".

| Key | Type | Default when absent | What it does |
| --- | --- | --- | --- |
| `default_privacy_profile` | text | local choice | The privacy profile applied to meetings with no client tag. Give the profile's name (`"Strict"`, `"Standard"`, `"Open"`, or one you created) or its id. A profile attached to a specific client still wins, because that is more specific than a workspace default. |
| `consent_level_floor` | text | local choice | The least a technician must do before recording. One of `self_only`, `notify`, `affirmative`, `per_speaker`. Acts as a floor unless locked. |
| `consent_enforcement` | text | local choice | What happens to a speaker who has not confirmed, under per-speaker consent. `flag_only` marks them; `strict` withholds their words from summaries, exports, chat, and the search index. `strict` cannot be relaxed by the local user. |
| `blocked_title_keywords` | list of text | local list | Words in a meeting title that block recording outright. Added to whatever the technician has set; they cannot remove yours. |
| `blocked_domains` | list of text | local list | Attendee email domains that block recording. Added to the local list the same way. |
| `retention_days` | whole number > 0 | local choice | The longest anything is kept. Acts as a ceiling: a shorter local window still applies, and a local "keep forever" is overridden. |
| `allowed_transcription_providers` | list of text | all allowed | The only transcription providers a technician may select. An empty list `[]` permits none. |
| `allowed_llm_providers` | list of text | all allowed | The only summary/chat model providers permitted. An empty list `[]` permits none. Refused at the point of use as well as at selection. |
| `telemetry` | `false` | off | Present for completeness. This app has no telemetry, so there is nothing to turn off. Setting it to `true` is ignored and reported; it cannot switch anything on. |
| `updates_enabled` | true/false | true | Whether the app may check for a newer release. With this `false`, no update request is made at all. |
| `locked` | list of key names | empty | Which of the keys above the local user cannot change. Naming a key you did not set is ignored and reported. |

### Provider names

Use these exact strings.

* Transcription: `localWhisper`, `parakeet`, `qwen`, `openai`, `remote`
* Models: `ollama`, `builtin-ai`, `openai`, `claude`, `groq`, `openrouter`,
  `custom-openai`

`ollama` and `builtin-ai` are the two that never leave the machine.

## A complete example

A managed services firm that wants everything local, every voice confirmed, nothing
kept past a quarter, and no update checks on production laptops:

```json
{
  "default_privacy_profile": "Strict",
  "consent_level_floor": "per_speaker",
  "consent_enforcement": "strict",
  "blocked_title_keywords": [
    "HR",
    "legal",
    "board",
    "termination",
    "disciplinary",
    "privileged"
  ],
  "blocked_domains": ["clinic.example", "lawfirm.example"],
  "retention_days": 90,
  "allowed_transcription_providers": ["localWhisper", "parakeet"],
  "allowed_llm_providers": ["ollama", "builtin-ai"],
  "telemetry": false,
  "updates_enabled": false,
  "locked": [
    "consent_level_floor",
    "consent_enforcement",
    "allowed_transcription_providers",
    "allowed_llm_providers",
    "updates_enabled"
  ]
}
```

What that machine now does: it will not send audio or transcript text to any cloud
service, because the only providers permitted run locally and those keys are locked.
It confirms every speaker and withholds anyone who does not confirm. It refuses to
record a meeting whose title mentions HR or legal, or that has an attendee from the
two listed domains. It deletes recordings and notes older than 90 days. It never
checks for updates. The technician can still make consent *stricter* than
per-speaker (there is nothing stricter) and retention *shorter* than 90 days, but
nothing looser.

## Deploying it

### Intune (Windows)

Two options, both fine.

**A platform script**, which is the simplest and works on any Windows edition:

1. Save the script below as `Set-MeetilyPolicy.ps1`, with your JSON pasted into the
   `$policy` here-string.
2. Intune admin centre → **Devices → Scripts and remediations → Platform scripts →
   Add → Windows 10 and later**.
3. Upload the script. Set **Run this script using the logged on credentials** to
   **No** (it needs to write to ProgramData) and **Run script in 64 bit PowerShell**
   to **Yes**.
4. Assign it to your technician device group.

```powershell
$folder = Join-Path $env:ProgramData 'meetily++'
New-Item -ItemType Directory -Path $folder -Force | Out-Null

$policy = @'
{
  "consent_level_floor": "per_speaker",
  "allowed_llm_providers": ["ollama", "builtin-ai"],
  "locked": ["consent_level_floor", "allowed_llm_providers"]
}
'@

$target = Join-Path $folder 'managed-config.json'
Set-Content -Path $target -Value $policy -Encoding UTF8

# Machine-wide, administrators-only write access.
$acl = Get-Acl $target
$acl.SetAccessRuleProtection($true, $false)
$acl.Access | ForEach-Object { $acl.RemoveAccessRule($_) | Out-Null }
foreach ($who in @('BUILTIN\Administrators', 'NT AUTHORITY\SYSTEM')) {
  $acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule(
    $who, 'FullControl', 'Allow')))
}
$acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule(
  'BUILTIN\Users', 'Read', 'Allow')))
Set-Acl -Path $target -AclObject $acl
```

**Or a Win32 app**, if you would rather have detection and reporting: wrap the same
script with `IntuneWinAppUtil`, use `powershell.exe -ExecutionPolicy Bypass -File
Set-MeetilyPolicy.ps1` as the install command, and use the file's existence at
`%ProgramData%\meetily++\managed-config.json` as the detection rule.

### Jamf (macOS)

1. Compose the policy JSON.
2. Jamf Pro → **Settings → Computer Management → Scripts → New**. Paste the script
   below, with your JSON in the heredoc.
3. **Computers → Policies → New**. Add the script, set **Trigger** to *Recurring
   check-in*, **Execution Frequency** to *Ongoing* (so a machine that drifts is
   corrected), and scope it to your technician smart group.

```bash
#!/bin/bash
set -euo pipefail

folder="/Library/Application Support/meetily++"
mkdir -p "$folder"

cat > "$folder/managed-config.json" <<'JSON'
{
  "consent_level_floor": "per_speaker",
  "allowed_llm_providers": ["ollama", "builtin-ai"],
  "locked": ["consent_level_floor", "allowed_llm_providers"]
}
JSON

chown root:wheel "$folder/managed-config.json"
chmod 644 "$folder/managed-config.json"
```

Jamf scripts run as root, so no extra privilege configuration is needed. If you
prefer a package to a script, the same file at the same path inside a `.pkg` payload
works identically.

### A generic RMM (or a plain script)

Any tool that runs a command as administrator or root will do. The whole job is
"create a folder, write a file, set permissions so users can read but not write".

Windows one-liner form, for an RMM that only accepts a single command:

```
powershell -NoProfile -ExecutionPolicy Bypass -Command "$d=Join-Path $env:ProgramData 'meetily++'; New-Item -ItemType Directory -Path $d -Force | Out-Null; Set-Content -Path (Join-Path $d 'managed-config.json') -Encoding UTF8 -Value '{\"consent_level_floor\":\"per_speaker\",\"locked\":[\"consent_level_floor\"]}'"
```

macOS and Linux one-liner form:

```bash
sudo sh -c 'mkdir -p "/Library/Application Support/meetily++" && printf %s "{\"consent_level_floor\":\"per_speaker\",\"locked\":[\"consent_level_floor\"]}" > "/Library/Application Support/meetily++/managed-config.json" && chmod 644 "/Library/Application Support/meetily++/managed-config.json"'
```

Run it on a schedule rather than once, so a machine someone has tampered with is put
back. Writing the same content twice is harmless.

## Checking it worked

On a target machine:

1. Open **Settings → Consent**. You should see a banner naming your file and listing
   what it set, with a padlock beside anything you locked.
2. Try to move a locked control. It should be read-only.
3. Open **Settings → Network activity**. Confirm the request list matches what your
   policy allows — for a local-only policy, nothing should appear there except model
   downloads.

If the banner says a file was found but could not be used, the reason is printed with
it; the usual cause is a trailing comma or a smart quote from a word processor. Edit
JSON in a plain text editor.

## Removing a policy

Delete the file. At the next launch the machine goes back to whatever the local user
had chosen, and the consent log records that no managed configuration was found. The
app does not overwrite local settings when a policy applies, so nothing has to be
restored by hand.
