import 'package:flutter/material.dart';
import 'design_tokens.dart';

/// Small, labelled actions stay usable with touch and large text settings.
class SessionActionButton extends StatelessWidget {
  final IconData icon;
  final String label;
  final VoidCallback? onPressed;
  final bool selected, busy;
  final String? hint;
  const SessionActionButton({
    super.key,
    required this.icon,
    required this.label,
    this.onPressed,
    this.selected = false,
    this.busy = false,
    this.hint,
  });
  @override
  Widget build(BuildContext context) => Tooltip(
    message: hint ?? label,
    child: OutlinedButton(
      onPressed: busy ? null : onPressed,
      style: OutlinedButton.styleFrom(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 12),
        minimumSize: const Size(0, SanserTokens.touchTarget),
        backgroundColor: selected ? SanserTokens.selected : null,
        foregroundColor: selected ? SanserTokens.accent : SanserTokens.textSoft,
      ),
      child: Row(
        children: [
          if (busy)
            const SizedBox.square(
              dimension: 18,
              child: CircularProgressIndicator(strokeWidth: 2),
            )
          else
            Icon(icon, size: 19),
          const SizedBox(width: 8),
          Expanded(
            child: Text(label, maxLines: 2, overflow: TextOverflow.ellipsis),
          ),
        ],
      ),
    ),
  );
}

class SessionActionGrid extends StatelessWidget {
  final List<Widget> children;
  const SessionActionGrid({super.key, required this.children});
  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) => Wrap(
      spacing: 8,
      runSpacing: 8,
      children: [
        for (final child in children)
          SizedBox(
            width: constraints.maxWidth < 210
                ? constraints.maxWidth
                : (constraints.maxWidth - 8) / 2,
            child: child,
          ),
      ],
    ),
  );
}
