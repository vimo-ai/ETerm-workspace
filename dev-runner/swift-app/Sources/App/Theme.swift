import SwiftUI

// MARK: - Sci-Fi Theme Colors

enum Theme {
    // Backgrounds - Pure black
    static let bgPrimary = Color(hex: "000000")
    static let bgSecondary = Color(hex: "0a0a0a")
    static let bgTertiary = Color(hex: "141414")
    static let bgHover = Color(hex: "1a1a1a")

    // Accent colors - Green theme
    static let accent = Color(hex: "2ad98d")      // rgb(42, 217, 141)
    static let accentAlt = Color(hex: "1fa86b")   // Darker green
    static let success = Color(hex: "2ad98d")     // Same as accent
    static let warning = Color(hex: "ffb800")     // Amber
    static let error = Color(hex: "ff4757")       // Red

    // Project type colors
    static let xcode = Color(hex: "5ac8fa")       // Blue for Xcode
    static let node = Color(hex: "8cc84b")        // Green for Node

    // Text
    static let textPrimary = Color(hex: "e8e8e8")
    static let textSecondary = Color(hex: "888888")
    static let textMuted = Color(hex: "555555")

    // Borders
    static let border = Color(hex: "2a2a2a")
    static let borderGlow = Color(hex: "2ad98d").opacity(0.3)
}

// MARK: - Color Extension

extension Color {
    init(hex: String) {
        let hex = hex.trimmingCharacters(in: CharacterSet.alphanumerics.inverted)
        var int: UInt64 = 0
        Scanner(string: hex).scanHexInt64(&int)
        let a, r, g, b: UInt64
        switch hex.count {
        case 6:
            (a, r, g, b) = (255, (int >> 16) & 0xFF, (int >> 8) & 0xFF, int & 0xFF)
        case 8:
            (a, r, g, b) = ((int >> 24) & 0xFF, (int >> 16) & 0xFF, (int >> 8) & 0xFF, int & 0xFF)
        default:
            (a, r, g, b) = (255, 0, 0, 0)
        }
        self.init(
            .sRGB,
            red: Double(r) / 255,
            green: Double(g) / 255,
            blue: Double(b) / 255,
            opacity: Double(a) / 255
        )
    }
}

// MARK: - Custom Components

struct SciFiButton: View {
    let title: String
    let icon: String
    var isActive: Bool = false
    var isDisabled: Bool = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: icon)
                    .font(.system(size: 12, weight: .medium))
                Text(title)
                    .font(.system(size: 12, weight: .medium, design: .monospaced))
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(
                RoundedRectangle(cornerRadius: 6)
                    .fill(isActive ? Theme.accent.opacity(0.15) : Theme.bgTertiary)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 6)
                    .stroke(isActive ? Theme.accent : Theme.border, lineWidth: 1)
            )
            .foregroundColor(isActive ? Theme.accent : Theme.textPrimary)
        }
        .buttonStyle(.plain)
        .disabled(isDisabled)
        .opacity(isDisabled ? 0.4 : 1)
    }
}

struct SciFiPicker<T: Hashable>: View {
    let label: String
    let icon: String
    @Binding var selection: T?
    let options: [T]
    let optionLabel: (T) -> String
    let optionIcon: ((T) -> String)?
    let optionBadge: ((T) -> (String, Color))?

    init(
        _ label: String,
        icon: String,
        selection: Binding<T?>,
        options: [T],
        optionLabel: @escaping (T) -> String,
        optionIcon: ((T) -> String)? = nil,
        optionBadge: ((T) -> (String, Color))? = nil
    ) {
        self.label = label
        self.icon = icon
        self._selection = selection
        self.options = options
        self.optionLabel = optionLabel
        self.optionIcon = optionIcon
        self.optionBadge = optionBadge
    }

    var body: some View {
        Menu {
            ForEach(options, id: \.self) { option in
                Button {
                    selection = option
                } label: {
                    HStack {
                        if let iconName = optionIcon?(option) {
                            Image(systemName: iconName)
                        }
                        Text(optionLabel(option))
                        if let badge = optionBadge?(option) {
                            Spacer()
                            Text(badge.0)
                                .font(.system(size: 10, weight: .medium, design: .monospaced))
                                .foregroundColor(badge.1)
                        }
                    }
                }
            }
        } label: {
            HStack(spacing: 8) {
                // Icon with badge color
                if let selected = selection, let badge = optionBadge?(selected) {
                    Image(systemName: icon)
                        .font(.system(size: 11))
                        .foregroundColor(badge.1)
                        .frame(width: 16)
                } else {
                    Image(systemName: icon)
                        .font(.system(size: 11))
                        .foregroundColor(Theme.accent)
                        .frame(width: 16)
                }

                VStack(alignment: .leading, spacing: 2) {
                    Text(label)
                        .font(.system(size: 9, weight: .medium, design: .monospaced))
                        .foregroundColor(Theme.textMuted)
                        .textCase(.uppercase)

                    if let selected = selection {
                        HStack(spacing: 6) {
                            Text(optionLabel(selected))
                                .font(.system(size: 12, weight: .medium, design: .monospaced))
                                .foregroundColor(Theme.textPrimary)

                            // Badge in selected display
                            if let badge = optionBadge?(selected) {
                                Text(badge.0)
                                    .font(.system(size: 9, weight: .bold, design: .monospaced))
                                    .foregroundColor(badge.1)
                                    .padding(.horizontal, 5)
                                    .padding(.vertical, 2)
                                    .background(badge.1.opacity(0.15))
                                    .clipShape(RoundedRectangle(cornerRadius: 3))
                            }
                        }
                    } else {
                        Text("Select...")
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundColor(Theme.textSecondary)
                    }
                }

                Spacer()

                Image(systemName: "chevron.down")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundColor(Theme.textMuted)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: 6)
                    .fill(Theme.bgTertiary)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 6)
                    .stroke(Theme.border, lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
    }
}

struct SciFiSidebarItem: View {
    let name: String
    let isSelected: Bool
    let onSelect: () -> Void
    let onRemove: () -> Void

    @State private var isHovered = false

    var body: some View {
        HStack(spacing: 10) {
            // Indicator
            RoundedRectangle(cornerRadius: 1)
                .fill(isSelected ? Theme.accent : Color.clear)
                .frame(width: 3, height: 20)

            // Icon
            Image(systemName: "cube.fill")
                .font(.system(size: 14))
                .foregroundColor(isSelected ? Theme.accent : Theme.textSecondary)

            // Name
            Text(name)
                .font(.system(size: 13, weight: isSelected ? .semibold : .regular, design: .monospaced))
                .foregroundColor(isSelected ? Theme.textPrimary : Theme.textSecondary)

            Spacer()

            // Remove button (on hover)
            if isHovered {
                Button {
                    onRemove()
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundColor(Theme.textMuted)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(isSelected ? Theme.bgHover : (isHovered ? Theme.bgTertiary : Color.clear))
        )
        .contentShape(Rectangle())
        .onTapGesture { onSelect() }
        .onHover { isHovered = $0 }
    }
}

struct GlowingBorder: ViewModifier {
    let color: Color
    let isActive: Bool

    func body(content: Content) -> some View {
        content
            .overlay(
                RoundedRectangle(cornerRadius: 8)
                    .stroke(isActive ? color : Theme.border, lineWidth: 1)
            )
            .shadow(color: isActive ? color.opacity(0.3) : .clear, radius: 8)
    }
}

extension View {
    func glowingBorder(color: Color = Theme.accent, isActive: Bool = true) -> some View {
        modifier(GlowingBorder(color: color, isActive: isActive))
    }
}
