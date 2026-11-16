-- Lua 5.1 engine mock for core versioning/identity/lifecycle contracts.
local requests, timers, now, response, errors = {}, {}, 0, nil, {}
local epoch = string.rep("a", 32)
local revision = 0
SCPRust = {}
file = {Read = function() return string.rep("b", 64) end}
string.Trim = function(s) return s end
function SysTime() return now end
function ErrorNoHalt(s) errors[#errors + 1] = s end
function Vector(x, y, z) return {x = x, y = y, z = z, vector = true} end
function isvector(v) return type(v) == "table" and v.vector == true end
function IsValid(e) return type(e) == "table" and e.valid end
local function entity(id)
    return {valid = true, id = id, generation = 7,
        GetCreationID = function(self) return self.generation end,
        EntIndex = function(self) return self.id end,
        GetPos = function(self) return Vector(self.id, 0, 0) end}
end
-- Minimal encoder to verify explicit [] construction as well as API lifecycle.
local function json(value)
    if type(value) == "string" then return string.format("%q", value) end
    if type(value) == "number" or type(value) == "boolean" then return tostring(value) end
    assert(type(value) == "table")
    local parts = {}
    if #value > 0 then
        for i, item in ipairs(value) do parts[i] = json(item) end
        return "[" .. table.concat(parts, ",") .. "]"
    end
    for key, item in pairs(value) do parts[#parts + 1] = json(key) .. ":" .. json(item) end
    return "{" .. table.concat(parts, ",") .. "}"
end
util = {TableToJSON = json, JSONToTable = function() return response end}
timer = {Create = function(name, _, _, fn) timers[name] = fn end,
    Remove = function(name) timers[name] = nil end}
function HTTP(request) requests[#requests + 1] = request; return true end
local function reply(value, code, index)
    response = value
    requests[index or #requests].success(code or 200, "mock JSON")
end
local function ver() return {epoch = epoch, revision = revision} end
local calls, result, failure = 0, nil, nil
local function cb(r, err) calls = calls + 1; result, failure = r, err end
local a, b = entity(1), entity(2)
local function key(e) return {id = e.id, generation = e.generation} end

dofile("addon/lua/scprust/sv_core.lua")
local core = SCPRust.Core
assert(not core.Publish({}, cb))
local function connect()
    assert(core.Connect(cb))
    assert(requests[#requests].method == "GET")
    reply({version = ver(), entities = 0, nodes = 0})
    assert(not failure)
end
local function publish(navigation)
    assert(core.Publish({{entity = a, faction = 1, alive = true}, {entity = b, faction = 2, alive = true}}, cb, navigation))
    revision = revision + 1
    reply({version = ver(), entities = 2, nodes = navigation and #navigation or 0})
    assert(not failure)
end
local function agent() return {entity = a, perceptionRadius = 100, attackRadius = 5,
    moveSpeed = 100, targetFactions = {2}} end
local function intent() return {actor = key(a), target = key(b), action = "attack_candidate",
    distance = 1, desired_velocity = {0, 0, 0}} end
connect()
assert(not core.Think({agent()}, cb), "connecting alone must not adopt someone else's entities")
publish({{id = 1, position = Vector(0, 0, 0), edges = {}}})
assert(requests[#requests].body:find('"edges":[]', 1, true), "empty graph adjacency must be []")
assert(core.Think({agent()}, cb))
assert(not core.Connect(cb), "serialize core operations")
reply({version = ver(), results = {intent()}})
assert(not failure and result[1].actor == a and result[1].target == b)
local before = calls
requests[#requests].failed("late")
assert(calls == before)

assert(core.Interest({{entity = a, radius = 100, maxEntities = 2}}, cb))
reply({version = ver(), results = {{actor = key(a), entities = {key(b)}, truncated = false}}})
assert(not failure and result[1].entities[1] == b)
assert(core.Routes({{start = 1, goal = 1}}, cb))
reply({version = ver(), results = {{status = "found", nodes = {1}, cost = 0, visited = 1}}})
assert(not failure and result[1].status == "found")

assert(core.Think({agent()}, cb))
now = 0.6
reply({version = ver(), results = {intent()}})
assert(failure == "stale core snapshot")
assert(not core.Think({agent()}, cb))
connect()
publish()
assert(core.Think({agent()}, cb))
b.generation = 8
local changed = intent()
changed.target.generation = 7
reply({version = ver(), results = {changed}})
assert(failure == "stale/invalid intent target")
connect()
publish()

assert(core.Routes({{start = 1, goal = 2}}, cb))
reply({version = {epoch = epoch, revision = revision + 1}, results = {} })
assert(failure == "invalid core batch/version")
connect()
assert(core.Publish({}, cb, {}))
assert(requests[#requests].body:find('"entities":[]', 1, true))
assert(requests[#requests].body:find('"navigation":[]', 1, true))
reply({}, 409)
assert(failure:find("409", 1, true))
assert(not core.Publish({}, cb), "conflict must invalidate connection state")

connect()
publish()
assert(core.Think({agent()}, cb))
for _, fn in pairs(timers) do fn(); break end
assert(failure:find("timeout", 1, true))
local timeoutCalls = calls
reply({version = ver(), results = {intent()}})
assert(calls == timeoutCalls)
assert(not core.Publish({}, cb), "timeout may conceal a commit; reconnect required")

connect()
assert(not core.Publish({{entity = a, faction = 1, alive = true}, {entity = a, faction = 1, alive = true}}, cb))
assert(not core.Publish({}, cb, {{id = 1, position = Vector(0, 0, 0), edges = {{to = 2, cost = 1}}}}))
assert(core.Publish({}, function() error("callback test") end))
revision = revision + 1
reply({version = ver()})
assert(#errors == 1, "user callback errors must release busy state")
assert(core.Connect(cb))
reply({version = ver()})
assert(not failure)
print("core bridge tests passed")
