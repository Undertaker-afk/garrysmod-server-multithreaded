PLUGIN.name = "Rust Worker Example"
PLUGIN.author = "SCPRust contributors"
PLUGIN.description = "Opt-in asynchronous Rust snapshot queries; no automatic hook replacements."

if SERVER then
    ix.command.Add("RustNearby", {
        description = "Query nearby players using the Rust worker (512-unit snapshot).",
        adminOnly = true,
        OnRun = function(self, client)
            if not SCPRust then return "SCPRust addon is not installed." end
            local accepted, reason = SCPRust.QueryRadius(player.GetAll(), {
                {center = client:GetPos(), radius = 512, maxHits = 512}
            }, function(results, err)
                if not IsValid(client) then return end
                if err then client:Notify("Rust worker: " .. err); return end
                client:Notify("Snapshot found " .. #results[1].entities .. " players (includes you).")
            end)
            if not accepted then return "Rust worker: " .. reason end
            return "Rust query submitted."
        end
    })
end

-- Connectivity/policy demo only: replaces the service's one world snapshot.
-- Do not run this alongside a production integration that owns that snapshot.
if SERVER then
    ix.command.Add("RustPlan", {
        description = "Preview a Rust NPC-policy decision without moving or damaging anything.",
        adminOnly = true,
        OnRun = function(self, client)
            if not SCPRust or not SCPRust.Core then return "SCPRust core addon is not installed." end
            local core = SCPRust.Core
            local function notify(message)
                if IsValid(client) then client:Notify(message) end
            end
            local function checked(accepted, reason)
                if not accepted then notify("Rust core: " .. tostring(reason)) end
            end
            local accepted, reason = core.Connect(function(_, err)
                if err then notify(err); return end
                if not IsValid(client) then return end
                local rows = {}
                for _, ply in ipairs(player.GetAll()) do
                    rows[#rows + 1] = {entity = ply, faction = ply == client and 1 or 2, alive = ply:Alive()}
                end
                checked(core.Publish(rows, function(_, publishError)
                    if publishError then notify(publishError); return end
                    if not IsValid(client) then return end
                    checked(core.Think({{entity = client, perceptionRadius = 2048,
                        attackRadius = 64, moveSpeed = 160, targetFactions = {2}}}, function(results, thinkError)
                        if thinkError then notify(thinkError); return end
                        notify("Rust policy preview: " .. results[1].action .. " (no gameplay action applied)")
                    end))
                end))
            end)
            if not accepted then return "Rust core: " .. tostring(reason) end
            return "Rust policy preview submitted."
        end
    })
end
