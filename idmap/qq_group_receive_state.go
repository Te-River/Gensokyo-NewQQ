package idmap

import (
	"fmt"
	"strings"
	"sync"

	"google.golang.org/protobuf/encoding/protowire"
)

const (
	// QQGroupPermitReceiveAllKey is stored under the QQ group OpenID section.
	QQGroupPermitReceiveAllKey = "permit_receive_all_qq_group_messages"

	qqGroupReceiveWindowSize = 10

	qqGroupMessageCreateEvent   uint64 = 1
	qqGroupAtMessageCreateEvent uint64 = 2
)

// qqGroupReceiveState is kept in memory in protobuf wire (TLV) form.
//
//	message QQGroupReceiveState {
//	  repeated QQGroupReceiveEntry recent = 1;
//	  bool permitted = 2;
//	  bool persisted_permitted = 3;
//	}
//
//	message QQGroupReceiveEntry {
//	  string message_id = 1;
//	  uint64 event = 2; // 1=GROUP_MESSAGE_CREATE, 2=GROUP_AT_MESSAGE_CREATE
//	}
type qqGroupReceiveState struct {
	Recent             []qqGroupReceiveEntry
	Permitted          bool
	PersistedPermitted bool
}

type qqGroupReceiveEntry struct {
	MessageID string
	Event     uint64
}

type qqGroupReceiveTracker struct {
	mu      sync.Mutex
	loaded  bool
	wireTLV []byte
}

var (
	qqGroupReceiveTrackers sync.Map
	qqGroupPermitRead      = ReadConfig
	qqGroupPermitWrite     = WriteConfig
)

// RecordQQGroupMessageReception updates the in-memory receive-all detection
// state for one QQ group. The recent MessageID window is protobuf wire/TLV data
// and is never persisted. The public idmap flag is read once per group to
// restore restart state, then written only when its value changes.
func RecordQQGroupMessageReception(groupOpenID, messageID string, groupMessageCreate bool) (bool, error) {
	groupOpenID = strings.TrimSpace(groupOpenID)
	messageID = strings.TrimSpace(messageID)
	if groupOpenID == "" || messageID == "" {
		return false, fmt.Errorf("QQ group reception state requires non-empty group OpenID and message ID")
	}

	value, _ := qqGroupReceiveTrackers.LoadOrStore(groupOpenID, &qqGroupReceiveTracker{})
	tracker := value.(*qqGroupReceiveTracker)
	tracker.mu.Lock()
	defer tracker.mu.Unlock()

	if !tracker.loaded {
		persisted, err := readPersistedQQGroupPermit(groupOpenID)
		if err != nil {
			return false, err
		}
		initial := qqGroupReceiveState{
			Permitted:          persisted,
			PersistedPermitted: persisted,
		}
		tracker.wireTLV = marshalQQGroupReceiveStateTLV(initial)
		tracker.loaded = true
	}

	state, err := unmarshalQQGroupReceiveStateTLV(tracker.wireTLV)
	if err != nil {
		tracker.loaded = false
		tracker.wireTLV = nil
		return false, fmt.Errorf("decode in-memory QQ group reception TLV: %w", err)
	}

	eventType := qqGroupAtMessageCreateEvent
	if groupMessageCreate {
		eventType = qqGroupMessageCreateEvent
	}
	state = advanceQQGroupReceiveState(state, messageID, eventType)
	tracker.wireTLV = marshalQQGroupReceiveStateTLV(state)

	if state.Permitted == state.PersistedPermitted {
		return state.Permitted, nil
	}

	flagValue := "0"
	if state.Permitted {
		flagValue = "1"
	}
	if err := qqGroupPermitWrite(groupOpenID, QQGroupPermitReceiveAllKey, flagValue); err != nil {
		// PersistedPermitted deliberately remains unchanged in the TLV so the
		// next event retries this transition without losing the MessageID window.
		return state.Permitted, fmt.Errorf("persist QQ group receive-all flag: %w", err)
	}

	state.PersistedPermitted = state.Permitted
	tracker.wireTLV = marshalQQGroupReceiveStateTLV(state)
	return state.Permitted, nil
}

func readPersistedQQGroupPermit(groupOpenID string) (bool, error) {
	value, err := qqGroupPermitRead(groupOpenID, QQGroupPermitReceiveAllKey)
	if err != nil {
		if isMissingQQGroupPermit(err) {
			return false, nil
		}
		return false, fmt.Errorf("read QQ group receive-all flag: %w", err)
	}

	switch strings.TrimSpace(value) {
	case "", "0", "false":
		return false, nil
	case "1", "true":
		return true, nil
	default:
		return false, fmt.Errorf("invalid QQ group receive-all flag %q", value)
	}
}

func advanceQQGroupReceiveState(state qqGroupReceiveState, messageID string, eventType uint64) qqGroupReceiveState {
	recent := make([]qqGroupReceiveEntry, 0, qqGroupReceiveWindowSize)
	for _, entry := range state.Recent {
		if entry.MessageID == "" || entry.MessageID == messageID {
			continue
		}
		recent = append(recent, entry)
	}
	recent = append(recent, qqGroupReceiveEntry{
		MessageID: messageID,
		Event:     eventType,
	})
	if len(recent) > qqGroupReceiveWindowSize {
		recent = recent[len(recent)-qqGroupReceiveWindowSize:]
	}

	switch eventType {
	case qqGroupAtMessageCreateEvent:
		state.Permitted = false
	case qqGroupMessageCreateEvent:
		// A persisted 1 remains enabled across restart while normal messages
		// continue. Once an AT event disables it, ten new distinct normal
		// MessageIDs are required to enable it again.
		if !state.Permitted {
			state.Permitted = len(recent) == qqGroupReceiveWindowSize
			if state.Permitted {
				for _, entry := range recent {
					if entry.Event != qqGroupMessageCreateEvent {
						state.Permitted = false
						break
					}
				}
			}
		}
	default:
		state.Permitted = false
	}

	state.Recent = recent
	return state
}

func marshalQQGroupReceiveStateTLV(state qqGroupReceiveState) []byte {
	var wire []byte
	for _, entry := range state.Recent {
		var entryWire []byte
		entryWire = protowire.AppendTag(entryWire, 1, protowire.BytesType)
		entryWire = protowire.AppendString(entryWire, entry.MessageID)
		entryWire = protowire.AppendTag(entryWire, 2, protowire.VarintType)
		entryWire = protowire.AppendVarint(entryWire, entry.Event)

		wire = protowire.AppendTag(wire, 1, protowire.BytesType)
		wire = protowire.AppendBytes(wire, entryWire)
	}
	if state.Permitted {
		wire = protowire.AppendTag(wire, 2, protowire.VarintType)
		wire = protowire.AppendVarint(wire, 1)
	}
	if state.PersistedPermitted {
		wire = protowire.AppendTag(wire, 3, protowire.VarintType)
		wire = protowire.AppendVarint(wire, 1)
	}
	return wire
}

func unmarshalQQGroupReceiveStateTLV(wire []byte) (qqGroupReceiveState, error) {
	var state qqGroupReceiveState
	for len(wire) > 0 {
		number, wireType, tagLength := protowire.ConsumeTag(wire)
		if tagLength < 0 {
			return qqGroupReceiveState{}, protowire.ParseError(tagLength)
		}
		wire = wire[tagLength:]

		switch number {
		case 1:
			if wireType != protowire.BytesType {
				return qqGroupReceiveState{}, fmt.Errorf("recent field has protobuf wire type %d", wireType)
			}
			entryWire, length := protowire.ConsumeBytes(wire)
			if length < 0 {
				return qqGroupReceiveState{}, protowire.ParseError(length)
			}
			entry, err := unmarshalQQGroupReceiveEntryTLV(entryWire)
			if err != nil {
				return qqGroupReceiveState{}, err
			}
			state.Recent = append(state.Recent, entry)
			wire = wire[length:]
		case 2, 3:
			if wireType != protowire.VarintType {
				return qqGroupReceiveState{}, fmt.Errorf("boolean field %d has protobuf wire type %d", number, wireType)
			}
			value, length := protowire.ConsumeVarint(wire)
			if length < 0 {
				return qqGroupReceiveState{}, protowire.ParseError(length)
			}
			if number == 2 {
				state.Permitted = value != 0
			} else {
				state.PersistedPermitted = value != 0
			}
			wire = wire[length:]
		default:
			length := protowire.ConsumeFieldValue(number, wireType, wire)
			if length < 0 {
				return qqGroupReceiveState{}, protowire.ParseError(length)
			}
			wire = wire[length:]
		}
	}
	return state, nil
}

func unmarshalQQGroupReceiveEntryTLV(wire []byte) (qqGroupReceiveEntry, error) {
	var entry qqGroupReceiveEntry
	for len(wire) > 0 {
		number, wireType, tagLength := protowire.ConsumeTag(wire)
		if tagLength < 0 {
			return qqGroupReceiveEntry{}, protowire.ParseError(tagLength)
		}
		wire = wire[tagLength:]

		switch number {
		case 1:
			if wireType != protowire.BytesType {
				return qqGroupReceiveEntry{}, fmt.Errorf("message_id field has protobuf wire type %d", wireType)
			}
			value, length := protowire.ConsumeString(wire)
			if length < 0 {
				return qqGroupReceiveEntry{}, protowire.ParseError(length)
			}
			entry.MessageID = value
			wire = wire[length:]
		case 2:
			if wireType != protowire.VarintType {
				return qqGroupReceiveEntry{}, fmt.Errorf("event field has protobuf wire type %d", wireType)
			}
			value, length := protowire.ConsumeVarint(wire)
			if length < 0 {
				return qqGroupReceiveEntry{}, protowire.ParseError(length)
			}
			entry.Event = value
			wire = wire[length:]
		default:
			length := protowire.ConsumeFieldValue(number, wireType, wire)
			if length < 0 {
				return qqGroupReceiveEntry{}, protowire.ParseError(length)
			}
			wire = wire[length:]
		}
	}
	return entry, nil
}

func isMissingQQGroupPermit(err error) bool {
	if err == nil {
		return false
	}
	message := strings.ToLower(err.Error())
	return strings.Contains(message, "does not exist") ||
		strings.Contains(message, "not found")
}
