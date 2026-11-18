-- stdout carries the command's exit status back to the resource. Log messages go to stderr.
on run argv
    set commandText to my quotedCommand(argv)
    try
        set commandOutput to do shell script commandText with administrator privileges
        if commandOutput is not "" then log commandOutput
        return 0
    on error errorText number errorCode
        return my commandFailed(errorText, errorCode)
    end try
end run

on quotedCommand(argv)
    set wordsQuoted to {}
    repeat with arg in argv
        set end of wordsQuoted to quoted form of (contents of arg)
    end repeat
    set AppleScript's text item delimiters to " "
    return wordsQuoted as text
end quotedCommand

on commandFailed(errorText, errorCode)
    -- User cancelled, or Authorization Services denied/cancelled authorization.
    if errorCode is in {-128, -60005, -60006} then
        log "furnish: Cancelled: administrator rights were not granted"
        return 12
    end if
    if errorText is not "" then log errorText
    if errorCode > 0 and errorCode < 256 then return errorCode
    return 1
end commandFailed
