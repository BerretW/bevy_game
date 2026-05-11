-- shared/main.lua — běží na obou stranách, jako "hello world" demo.

log_info(string.format(
    '[%s] hello! loaded on %s side, isolation verified (Core is %s)',
    RESOURCE_ID,
    SIDE,
    -- `Core` byl definován v sandboxu pro core/init; v tomto sandboxu musí být nil,
    -- protože sandboxy jsou izolované. Slouží jako sanity check.
    tostring(Core)
))
if SIDE == "client" then
    local existing = World.GetHandlesByModel("mutant")
    local mutant = existing[1]

    -- Hot-reload safe: pokud už mutant existuje, znovu ho nespawnuj.
    -- Pokud jich je víc, nech první a zbytek smaž.
    if mutant then
        for i = 2, #existing do
            World.DeleteObject(existing[i])
        end
    else
        mutant = World.SpawnLocalObject("mutant", {x=3, y=1, z=3}, {x=0, y=0, z=0})
    end

    Engine.RequestModel("mutant")

    CreateThread(function()
        -- Počkej, až bude model i metadata clipů dostupná.
        local attempts = 0
        while attempts < 600 do
            local loaded = Engine.HasModelLoaded("mutant")
            local clip_count = Engine.GetModelClipCount("mutant")
            if loaded and clip_count > 0 then
                break
            end
            attempts = attempts + 1
            Wait(100)
        end

        local clip_names = Engine.GetModelClipNames("mutant")
        if not clip_names or #clip_names == 0 then
            log_warn("[hello] mutant: no animation clips available")
            return
        end

        local i = 1
        while true do
            local clip = clip_names[i]
            World.PlayAnimation(mutant, clip, true, 1.0, 0.15)
            log_info(string.format("[hello] mutant playing clip %d/%d: %s", i, #clip_names, tostring(clip)))

            Wait(5000)
            i = i + 1
            if i > #clip_names then
                i = 1
            end
        end
    end)
end