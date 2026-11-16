-- Versioned, explicitly opt-in gameplay core. No automatic timers or movement.
if not SCPRust or SCPRust.Core then return end
local Core = {}
SCPRust.Core = Core
local token = string.Trim(file.Read("scprust_token.txt", "DATA") or "")
local baseURL = "http://127.0.0.1:27185/v1/"
local version, capturedAt, references
local busy, sequence = false, 0
local MAX_AGE = 0.5

local function finite(n, lo, hi)
    return type(n) == "number" and n == n and n >= lo and n <= hi
end
local function integer(n, lo, hi) return finite(n, lo, hi) and n % 1 == 0 end
local function dense(t, max)
    if type(t) ~= "table" then return false end
    local count = 0
    for k in pairs(t) do
        count = count + 1
        if count > max or not integer(k, 1, max) then return false end
    end
    for i = 1, count do if t[i] == nil then return false end end
    return true
end
local function position(v)
    if not isvector(v) or not finite(v.x, -1000000, 1000000)
        or not finite(v.y, -1000000, 1000000) or not finite(v.z, -1000000, 1000000) then return nil end
    return {v.x, v.y, v.z}
end
local function validVersion(v)
    return type(v) == "table" and type(v.epoch) == "string" and #v.epoch == 32
        and v.epoch:match("^[a-f0-9]+$") and integer(v.revision, 0, 4294967295)
end
local function sameVersion(a, b)
    return validVersion(a) and validVersion(b) and a.epoch == b.epoch and a.revision == b.revision
end
local function reset() version, references, capturedAt = nil, nil, nil end
local function keyEqual(a, b)
    return type(a) == "table" and type(b) == "table" and a.id == b.id and a.generation == b.generation
end
local function resolve(key)
    if not references or type(key) ~= "table" or not integer(key.id, 1, 4294967295)
        or not integer(key.generation, 0, 4294967295) then return nil end
    local ref = references[key.id]
    if ref and ref.generation == key.generation and IsValid(ref.entity)
        and ref.entity:GetCreationID() == ref.generation then return ref.entity end
end
local function keyFor(entity)
    if not IsValid(entity) or not references then return nil end
    local key = {id = entity:EntIndex(), generation = entity:GetCreationID()}
    if resolve(key) ~= entity then return nil end
    return key
end
local function arrayJSON(rows)
    -- Encode each nonempty object normally, but explicitly represent empty arrays
    -- as []. GMod's generic encoder otherwise represents an empty table as {}.
    return "[" .. table.concat(rows, ",") .. "]"
end
local function rejectResponse(callback, message)
    reset()
    callback(nil, message)
end

-- Once accepted, exactly one callback (unless Lua shuts down). Immediate HTTP
-- submission failure may invoke it before returning. Rejected calls never do.
local function send(path, body, callback)
    if type(callback) ~= "function" then return false, "callback required" end
    if busy then return false, "core request already in flight" end
    if #token < 32 or not token:match("^[a-zA-Z0-9]+$") then return false, "missing/invalid worker token" end
    if body and #body > 2 * 1024 * 1024 then return false, "core body too large" end
    busy, sequence = true, sequence + 1
    local timerName = "SCPRust.Core." .. sequence
    local done = false
    local function finish(result, err)
        if done then return end
        done, busy = true, false
        timer.Remove(timerName)
        -- Even a timed-out publication may have committed. Never guess a version.
        if err then reset() end
        local ok, message = xpcall(function() callback(result, err) end, debug.traceback)
        if not ok then ErrorNoHalt("[SCPRust.Core] callback: " .. tostring(message) .. "\n") end
    end
    timer.Create(timerName, 2, 1, function() finish(nil, "core timeout; reconnect required") end)
    local submitted = HTTP({
        url = baseURL .. path, method = body and "POST" or "GET",
        type = "application/json", body = body,
        headers = {Authorization = "Bearer " .. token}, timeout = 2,
        failed = function(reason) finish(nil, "core HTTP failed: " .. tostring(reason)) end,
        success = function(code, response)
            if done then return end
            if code ~= 200 then finish(nil, "core HTTP " .. tostring(code) .. "; reconnect required"); return end
            -- 128 * 512 generation-bearing keys can exceed 2 MiB at u32 maxima.
            if type(response) ~= "string" or #response > 4 * 1024 * 1024 then
                finish(nil, "invalid core response size"); return
            end
            local decoded = util.JSONToTable(response)
            if type(decoded) ~= "table" then finish(nil, "invalid core JSON"); return end
            finish(decoded, nil)
        end
    })
    if not submitted then finish(nil, "core HTTP could not start (check -allowlocalhttp)") end
    return true
end

function Core.Connect(callback)
    if type(callback) ~= "function" then return false, "callback required" end
    return send("world", nil, function(info, err)
        if err then callback(nil, err); return end
        if not validVersion(info.version) then rejectResponse(callback, "invalid world version"); return end
        reset()
        version = {epoch = info.version.epoch, revision = info.version.revision}
        callback(info, nil)
    end)
end

-- rows: {{entity = ent, faction = uint16, alive = boolean}, ...}
-- navigation: nil retains the graph; {} clears it. Nodes use Vector positions
-- and directed edges {{to = nodeID, cost = positiveNumber}, ...}.
function Core.Publish(rows, callback, navigation)
    if type(callback) ~= "function" then return false, "callback required" end
    if busy then return false, "core request already in flight" end
    if not version then return false, "connect first" end
    if not dense(rows, 8192) then return false, "invalid entity array" end
    local started, nextRefs, encoded = SysTime(), {}, {}
    for i, row in ipairs(rows) do
        if type(row) ~= "table" or not IsValid(row.entity) or not integer(row.faction, 0, 65535)
            or type(row.alive) ~= "boolean" then return false, "invalid snapshot entity" end
        local ent = row.entity
        local id, generation, pos = ent:EntIndex(), ent:GetCreationID(), position(ent:GetPos())
        if not integer(id, 1, 4294967295) or not integer(generation, 0, 4294967295)
            or not pos or nextRefs[id] then return false, "invalid/duplicate entity identity or position" end
        nextRefs[id] = {entity = ent, generation = generation}
        encoded[i] = util.TableToJSON({key = {id = id, generation = generation}, position = pos,
            faction = row.faction, alive = row.alive})
    end
    local graph = ""
    if navigation ~= nil then
        if not dense(navigation, 4096) then return false, "invalid navigation array" end
        local nodes, nodeIDs, edgeCount = {}, {}, 0
        for i, node in ipairs(navigation) do
            if type(node) ~= "table" or not integer(node.id, 1, 4294967295) or nodeIDs[node.id]
                or not position(node.position) or not dense(node.edges, 32768) then
                return false, "invalid navigation node"
            end
            nodeIDs[node.id] = true
            local edges, targets = {}, {}
            for j, edge in ipairs(node.edges) do
                edgeCount = edgeCount + 1
                if edgeCount > 32768 or type(edge) ~= "table" or not integer(edge.to, 1, 4294967295)
                    or not finite(edge.cost, 0, 1000000000) or edge.cost == 0 or targets[edge.to] then
                    return false, "invalid navigation edge"
                end
                targets[edge.to] = true
                edges[j] = util.TableToJSON({to = edge.to, cost = edge.cost})
            end
            nodes[i] = '{"id":' .. node.id .. ',"position":' .. util.TableToJSON(position(node.position))
                .. ',"edges":' .. arrayJSON(edges) .. '}'
        end
        for _, node in ipairs(navigation) do
            for _, edge in ipairs(node.edges) do
                if not nodeIDs[edge.to] then return false, "unknown navigation edge target" end
            end
        end
        graph = ',"navigation":' .. arrayJSON(nodes)
    end
    local base = {epoch = version.epoch, revision = version.revision}
    local body = '{"base":' .. util.TableToJSON(base) .. ',"entities":' .. arrayJSON(encoded) .. graph .. '}'
    return send("world", body, function(info, err)
        if err then callback(nil, err); return end
        if not validVersion(info.version) or info.version.epoch ~= base.epoch
            or info.version.revision ~= base.revision + 1 then
            rejectResponse(callback, "invalid publication version"); return
        end
        version = {epoch = info.version.epoch, revision = info.version.revision}
        references, capturedAt = nextRefs, started
        callback(info, nil)
    end)
end

local function batch(path, items, callback, needsFreshSnapshot, decode)
    if type(callback) ~= "function" then return false, "callback required" end
    if not version or not references then return false, "publish a snapshot first" end
    if needsFreshSnapshot and SysTime() - capturedAt > MAX_AGE then return false, "publish a fresh snapshot" end
    local expected = {epoch = version.epoch, revision = version.revision}
    local body = '{"version":' .. util.TableToJSON(expected) .. ',"items":' .. arrayJSON(items) .. '}'
    return send(path, body, function(reply, err)
        if err then callback(nil, err); return end
        if not sameVersion(reply.version, expected) or not dense(reply.results, 128) or #reply.results ~= #items then
            rejectResponse(callback, "invalid core batch/version"); return
        end
        if needsFreshSnapshot and (not capturedAt or SysTime() - capturedAt > MAX_AGE) then
            rejectResponse(callback, "stale core snapshot"); return
        end
        local results, reason = decode(reply.results)
        if not results then rejectResponse(callback, reason); return end
        callback(results, nil)
    end)
end

-- Requests are custom policy, not the engine's NPC AI. Suggestions do not move,
-- attack, trace, or authorize anything; the caller must validate and apply them.
function Core.Think(agents, callback)
    if not dense(agents, 128) or #agents == 0 then return false, "invalid agent batch" end
    local encoded, keys = {}, {}
    for i, agent in ipairs(agents) do
        if type(agent) ~= "table" then return false, "invalid agent" end
        local key = keyFor(agent.entity)
        if not key or not finite(agent.perceptionRadius, 0, 1000000)
            or not finite(agent.attackRadius, 0, agent.perceptionRadius)
            or not finite(agent.moveSpeed, 0, 10000) or not dense(agent.targetFactions, 32)
            or #agent.targetFactions == 0 then return false, "invalid agent configuration/identity" end
        for _, faction in ipairs(agent.targetFactions) do
            if not integer(faction, 0, 65535) then return false, "invalid target faction" end
        end
        local target = agent.currentTarget and keyFor(agent.currentTarget) or nil
        if agent.currentTarget and not target then return false, "unknown current target identity" end
        keys[i] = key
        encoded[i] = util.TableToJSON({actor = key, perception_radius = agent.perceptionRadius,
            attack_radius = agent.attackRadius, move_speed = agent.moveSpeed,
            target_factions = agent.targetFactions, current_target = target})
    end
    return batch("think", encoded, callback, true, function(results)
        local resolved = {}
        for i, result in ipairs(results) do
            if type(result) ~= "table" or not keyEqual(result.actor, keys[i]) or not resolve(result.actor)
                or not dense(result.desired_velocity, 3) or #result.desired_velocity ~= 3 then
                return nil, "invalid core intent/actor"
            end
            for _, n in ipairs(result.desired_velocity) do
                if not finite(n, -10000, 10000) then return nil, "invalid intent velocity" end
            end
            local target = result.target and resolve(result.target) or nil
            if result.action == "idle" then
                if result.target or result.distance then return nil, "invalid idle intent" end
            elseif result.action == "chase" or result.action == "attack_candidate" then
                if not target or not finite(result.distance, 0, 4000000) then return nil, "stale/invalid intent target" end
            else return nil, "invalid intent action" end
            resolved[i] = {actor = resolve(result.actor), target = target, action = result.action,
                desiredVelocity = Vector(unpack(result.desired_velocity)), distance = result.distance}
        end
        return resolved
    end)
end

function Core.Interest(observers, callback)
    if not dense(observers, 128) or #observers == 0 then return false, "invalid observer batch" end
    local encoded, keys, limits = {}, {}, {}
    for i, observer in ipairs(observers) do
        if type(observer) ~= "table" then return false, "invalid observer" end
        local key = keyFor(observer.entity)
        local maxEntities = observer.maxEntities or 512
        if not key or not finite(observer.radius, 0, 1000000) or not integer(maxEntities, 1, 512) then
            return false, "invalid observer configuration/identity"
        end
        keys[i], limits[i] = key, maxEntities
        encoded[i] = util.TableToJSON({actor = key, radius = observer.radius, max_entities = maxEntities})
    end
    return batch("interest", encoded, callback, true, function(results)
        local resolved = {}
        for i, result in ipairs(results) do
            if type(result) ~= "table" or not keyEqual(result.actor, keys[i]) or not resolve(result.actor)
                or not dense(result.entities, limits[i]) or type(result.truncated) ~= "boolean" then
                return nil, "invalid interest result"
            end
            local entities, seen = {}, {}
            for j, key in ipairs(result.entities) do
                local entity = resolve(key)
                if not entity or seen[entity] or keyEqual(key, keys[i]) then return nil, "stale/invalid interest entity" end
                seen[entity], entities[j] = true, entity
            end
            resolved[i] = {actor = resolve(result.actor), entities = entities, truncated = result.truncated}
        end
        return resolved
    end)
end

function Core.Routes(queries, callback)
    if not dense(queries, 128) or #queries == 0 then return false, "invalid route batch" end
    local encoded, expected = {}, {}
    for i, query in ipairs(queries) do
        if type(query) ~= "table" or not integer(query.start, 1, 4294967295)
            or not integer(query.goal, 1, 4294967295) or not integer(query.maxVisited or 4096, 1, 4096) then
            return false, "invalid route query"
        end
        expected[i] = {start = query.start, goal = query.goal, max_visited = query.maxVisited or 4096}
        encoded[i] = util.TableToJSON(expected[i])
    end
    return batch("routes", encoded, callback, false, function(results)
        for i, result in ipairs(results) do
            if type(result) ~= "table" or not dense(result.nodes, 256)
                or not integer(result.visited, 0, expected[i].max_visited) then return nil, "invalid route result" end
            for _, id in ipairs(result.nodes) do
                if not integer(id, 1, 4294967295) then return nil, "invalid route node" end
            end
            if result.status == "found" then
                if #result.nodes == 0 or result.nodes[1] ~= expected[i].start
                    or result.nodes[#result.nodes] ~= expected[i].goal or not finite(result.cost, 0, 4096000000000) then
                    return nil, "invalid found route"
                end
            elseif result.status == "unreachable" or result.status == "budget_exceeded" or result.status == "path_too_long" then
                if #result.nodes ~= 0 or result.cost ~= nil then return nil, "invalid failed route" end
            else return nil, "invalid route status" end
        end
        return results
    end)
end
