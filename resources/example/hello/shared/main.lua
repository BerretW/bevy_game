-- shared/main.lua — běží na obou stranách, jako "hello world" demo.

log_info(string.format(
    '[%s] hello! loaded on %s side, isolation verified (Core is %s)',
    RESOURCE_ID,
    SIDE,
    -- `Core` byl definován v sandboxu pro core/init; v tomto sandboxu musí být nil,
    -- protože sandboxy jsou izolované. Slouží jako sanity check.
    tostring(Core)
))
World.SpawnLocalObject("mutant", {x=3, y=1, z=3}, {x=90, y=0, z=0})