export function getNinjaLeagueUrl(league: string) {
  // poe.ninja league url slugs: "Standard" -> "standard", "Mirage" -> "mirage",
  // "Hardcore Mirage" -> "miragehc"
  if (league.startsWith('Hardcore ')) {
    return `${league.substring('Hardcore '.length).toLowerCase().replace(/\s+/g, '')}hc`;
  }
  return league.toLowerCase().replace(/\s+/g, '');
}

export function getNinjaTypeUrl(type: string) {
  return `${type.replace(/([a-zA-Z])(?=[A-Z])/g, '$1-').toLowerCase()}s`
    .replace('prophecys', 'prophecies')
    .replace('accessorys', 'accessories')
    .replace('currencys', 'currency')
    .replace('fragmentss', 'fragments');
}
