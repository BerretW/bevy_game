-- ESC menu využívající UI.Window framework.

local menu = UI.Window({
    title = "PAUSED",
    width = 0.26,
})

menu:Button("Resume",          function() menu:Close(); Engine.SetCursorLocked(true) end)
menu:Button("Disconnect",      function() menu:Close(); Engine.Disconnect()           end, "danger")
menu:Button("Quit to Desktop", function() Engine.Quit()                              end, "danger")

RegisterEvent("input:state", function(payload)
    if not payload then return end
    if payload.keys_just and payload.keys_just.options_menu then
        menu:Toggle()
        Engine.SetCursorLocked(not menu:IsOpen())
    end
end)

CreateThread(function()
    while true do
        local f = menu:GetFade()
        if f > 0.002 then
            Gui.DrawRect(0.5, 0.5, 1.0, 1.0, 0, 0, 0, math.floor(110 * f))
        end
        menu:Render()
        Wait(0)
    end
end)
