// Disposable hosted-Mac CI driver, not application behavior or native proof.
// Readiness callbacks may poll read-only state; mutation callbacks run once.
export type PickerDriver = {
  storageIdle: () => Promise<void>;
  change: () => Promise<void>;
  picker: () => Promise<string>;
  openGo: () => Promise<void>;
  goSheet: () => Promise<string>;
  navigate: (path: string) => Promise<void>;
};
export async function prepareStorageChange(driver: PickerDriver, path: string) {
  await driver.storageIdle();
  await driver.change();
  const beforeGo = await driver.picker();
  await driver.openGo();
  const goSheet = await driver.goSheet();
  await driver.navigate(path);
  const beforeChoose = await driver.picker();
  return {beforeGo, goSheet, beforeChoose};
}

const pickerScript = `
on run argv
  set targetPID to (item 1 of argv) as integer
  set operation to item 3 of argv
  set goMode to operation is "inspect-go" or operation is "navigate-go"
  tell application "System Events"
    set owned to first application process whose unix id is targetPID
    tell owned
      if (count of windows) is 0 then error "Owned process has no AX windows; frontmost=" & (frontmost as text)
      set outerWindow to window 1
      set picker to outerWindow
      set depth to 0
      set goParent to missing value
      set trace to ""
      repeat
        set panelName to ""
        try
          if name of picker is not missing value then set panelName to name of picker as text
        end try
        set panelRole to role of picker as text
        set labels to name of every button of picker
        set trace to trace & "depth=" & depth & ";role=" & panelRole & ";title=" & panelName & ";buttons="
        repeat with labelText in labels
          set trace to trace & (labelText as text) & "|"
        end repeat
        set trace to trace & linefeed
        set childSheets to sheets of picker
        if (count of childSheets) is 0 then exit repeat
        if (count of childSheets) is not 1 or depth is 4 then error "Unsupported owned AX sheet tree: " & trace
        set goParent to picker
        set picker to item 1 of childSheets
        set depth to depth + 1
      end repeat
      -- Failure diagnostics must not depend on the recursive selector succeeding.
      if operation is "diagnose" then
        set directChildren to UI elements of picker
        set trace to trace & "immediate-child-count=" & (count of directChildren) & linefeed
        set childLimit to count of directChildren
        if childLimit > 16 then set childLimit to 16
        repeat with childIndex from 1 to childLimit
          set directChild to item childIndex of directChildren
          set trace to trace & "child-role=" & (role of directChild as text) & ";next-roles="
          set nextRoles to role of every UI element of directChild
          if (count of nextRoles) > 32 then
            set trace to trace & "over-32"
          else
            repeat with nextRole in nextRoles
              set trace to trace & (nextRole as text) & "|"
            end repeat
          end if
          set trace to trace & linefeed
        end repeat
        return trace
      end if
      if goMode then
        if depth is 0 or goParent is missing value then error "Owned Go to Folder sheet not ready: " & trace
        set goSheet to picker
        if (role of goSheet as text) is not "AXSheet" then error "Unexpected owned Go to Folder role: " & trace
        set picker to goParent
        set depth to depth - 1
        set panelRole to role of picker as text
        set panelName to ""
        if name of picker is not missing value then set panelName to name of picker as text
      end if
      if depth > 0 and (name of outerWindow as text) is not "Apparent — UI foundation" then error "Picker has unexpected parent: " & trace
      if panelRole is not "AXSheet" and panelRole is not "AXWindow" then error "Unexpected picker role: " & trace
      -- A sheet can have no title. A named picker must expose the shipped title.
      if panelName is not "" and panelName is not "Choose capture location" then error "Unexpected picker title: " & trace
      -- Observed macOS 26 NSOpenPanel: one AXSplitGroup immediately under the
      -- sheet, with the footer buttons directly under that group. Do not walk
      -- the file-browser descendants (which can expose an entire volume).
      set controls to picker
      if (count of buttons of controls) is 0 then
        set containers to UI elements of picker
        if (count of containers) is not 1 then error "Unsupported picker control containers: " & trace
        set controls to item 1 of containers
        if (role of controls as text) is not "AXSplitGroup" then error "Unsupported picker control container role: " & trace
      end if
      set chooseMatches to {}
      set cancelMatches to {}
      set trace to trace & "footer-buttons="
      set footerButtons to buttons of controls
      if (count of footerButtons) > 16 then error "Oversized picker footer: " & trace
      repeat with foundButton in footerButtons
        set buttonName to name of foundButton
        if buttonName is not missing value then
          set trace to trace & (buttonName as text) & "|"
          if buttonName is "Choose" then set end of chooseMatches to contents of foundButton
          if buttonName is "Cancel" then set end of cancelMatches to contents of foundButton
        end if
      end repeat
      set trace to trace & linefeed
      if (count of chooseMatches) is not 1 or (count of cancelMatches) is not 1 then error "Owned native picker controls missing or ambiguous: " & trace
      set chooseButton to item 1 of chooseMatches
      set cancelButton to item 1 of cancelMatches
      if not goMode and not (enabled of chooseButton) then error "Native Choose not enabled: " & trace
      set xy to position of outerWindow
      set wh to size of outerWindow
      if goMode then
        if not frontmost then error "Focus left owned Electron process"
        set goSheets to sheets of picker
        if (count of goSheets) is not 1 then error "Owned Go to Folder sheet missing or ambiguous"
        set goSheet to item 1 of goSheets
        set pathFields to text fields of goSheet
        if (count of pathFields) is not 1 then error "Owned Go to Folder path field missing or ambiguous"
        set pathField to item 1 of pathFields
        if not (enabled of pathField) then error "Owned Go to Folder path field not ready"
        if operation is "navigate-go" then
          -- argv, not script interpolation: spaces, quotes and accents remain data.
          set value of pathField to (item 2 of argv)
          if (value of pathField as text) is not (item 2 of argv) then error "Go to Folder path readback differs"
          -- Preserve the bounded autocomplete interval and exactly one Return.
          delay 1
          if not frontmost then error "Focus left owned Electron process"
          -- Recheck the exact owned modal before Return. Never let a vanished
          -- Go sheet turn a navigation key into the parent picker's Choose.
          set currentGoSheets to sheets of picker
          if (count of currentGoSheets) is not 1 then error "Owned Go to Folder sheet disappeared before navigation"
          set currentFields to text fields of item 1 of currentGoSheets
          if (count of currentFields) is not 1 then error "Owned Go to Folder field changed before navigation"
          if (value of item 1 of currentFields as text) is not (item 2 of argv) then error "Go to Folder path changed before navigation"
          key code 36
        end if
      else if operation is "open-go" or operation is "choose" or operation is "cancel" then
        set frontmost to true
        if not frontmost then error "Owned Electron process is not frontmost"
        if operation is "open-go" then
          keystroke "g" using {command down, shift down}
        else if operation is "choose" then
          click chooseButton
        else
          click cancelButton
        end if
      end if
      return trace & "bounds=" & (item 1 of xy) & "," & (item 2 of xy) & "," & (item 1 of wh) & "," & (item 2 of wh)
    end tell
  end tell
end run
`;

// Preserve the last AX query in subprocess stderr when a hosted runner stalls.
// Only logging is inserted: ownership checks, queries and one-shot mutations
// remain in the one selector above, with the caller's existing deadline.
export const panelScript = pickerScript.split('\n').flatMap((line, index) => {
  const query = /^\s*(set|if)\b/.test(line) &&
    /\b(application process|windows?|name|role|buttons|sheets|UI elements|position|size|enabled|frontmost|text fields|value)\b/.test(line);
  return query ? [`${line.match(/^\s*/)![0]}log "AX probe source-line=${index + 1}"`, line] : [line];
}).join('\n');
